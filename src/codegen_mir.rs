//! The fast backend, from MIR: `mir::Program` → Cranelift IR → object file.
//! Must behave exactly like `cgen_mir`.
//!
//! One Cranelift block per MIR block, emitted in reverse post-order. Locals
//! with `in_memory == false` are SSA `Variable`s (so numeric code is the same
//! SSA as it was before MIR); the others get a stack slot. The calling
//! convention is the HIR backend's (see `codegen`): strings and structs are
//! passed by address, results of those types through a hidden first pointer.
//! A `Move` of such an argument passes the source's own address, a `Copy`
//! first copies it, because the callee owns what it is given.
//!
//! A struct that needs it gets a drop function (`drachm_drop_S(ptr)`: its
//! `drop()` first, then its fields in declaration order) and, if it holds
//! strings, a copy function (`drachm_copy_S(dst, src)`), each defined once.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, StackSlotData, StackSlotKind, TrapCode, UserFuncName, Value};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::codegen_common::{cl_type, emit_c_main, err, foreign_signature, in_memory, is_signed, key_tag, map_ops_type, signature, sysv_struct, Eightbyte, MEM, STR_CAP, STR_LEN};
use crate::mir::MapFn;
use crate::hir::{self, BinOp, STR_SIZE};
use crate::mir::drops::{copy_glue_types, drop_glue_types, glue_name, holds_string};
use crate::mir::needs_drop;
use crate::mir::traversal::reverse_postorder;
use crate::mir::ty::operand_ty;
use crate::mir::*;
use crate::types::Type;

type ClType = types::Type;
type ClBlock = cranelift_codegen::ir::Block;

pub fn compile(hir: &hir::Program, mir: &Program) -> Result<Vec<u8>, String> {
    let mut flags = settings::builder();
    flags.set("is_pic", "true").map_err(err)?;
    flags.set("opt_level", "speed").map_err(err)?;
    // Lets 128-bit integers appear in (Drachm-to-Drachm) function signatures.
    flags.set("enable_llvm_abi_extensions", "true").map_err(err)?;
    let isa = cranelift_native::builder().map_err(err)?.finish(settings::Flags::new(flags)).map_err(err)?;
    let builder = ObjectBuilder::new(isa, "drachm", cranelift_module::default_libcall_names()).map_err(err)?;
    let mut module = ObjectModule::new(builder);

    let ptr = module.target_config().pointer_type();
    let mut ids = Vec::new();
    for (i, func) in hir.funcs.iter().enumerate() {
        if let Some(symbol) = hir.foreign.get(&i) {
            // A C function: the real symbol, found by the linker.
            let s = foreign_signature(&module, hir, func);
            ids.push(module.declare_function(symbol, Linkage::Import, &s).map_err(err)?);
            continue;
        }
        let s = signature(&module, func);
        // A function with no body is a test block that this build leaves out: declared, never defined.
        let linkage = if mir.bodies[i].is_some() { Linkage::Local } else { Linkage::Import };
        ids.push(module.declare_function(&func.symbol, linkage, &s).map_err(err)?);
    }

    // Glue functions: declared up front so any function can call them.
    let mut drop_ids: HashMap<Type, FuncId> = HashMap::new();
    let mut copy_ids: HashMap<Type, FuncId> = HashMap::new();
    for t in drop_glue_types(hir) {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        drop_ids.insert(t, module.declare_function(&format!("drachm_drop_{}", glue_name(t)), Linkage::Local, &sig).map_err(err)?);
    }
    for t in copy_glue_types(hir) {
        let mut sig = module.make_signature();
        sig.params.extend([AbiParam::new(ptr), AbiParam::new(ptr)]);
        copy_ids.insert(t, module.declare_function(&format!("drachm_copy_{}", glue_name(t)), Linkage::Local, &sig).map_err(err)?);
    }

    let vt_ids = define_vtables(&mut module, hir, &ids)?;
    // The hash and equality functions of each key tag used by a map (S6D), declared up front.
    let mut key_ids: HashMap<String, (FuncId, FuncId, Type)> = HashMap::new();
    for t in hir.collections.iter().copied().filter(|t| t.is_map()) {
        let tag = key_tag(t.map_parts().expect("a map").0);
        if key_ids.contains_key(&tag) {
            continue;
        }
        let mut hs = module.make_signature();
        hs.params.push(AbiParam::new(ptr));
        hs.returns.push(AbiParam::new(types::I64));
        let mut es = module.make_signature();
        es.params.extend([AbiParam::new(ptr), AbiParam::new(ptr)]);
        es.returns.push(AbiParam::new(types::I8));
        let h = module.declare_function(&format!("drachm_keyhash_{tag}"), Linkage::Local, &hs).map_err(err)?;
        let e = module.declare_function(&format!("drachm_keyeq_{tag}"), Linkage::Local, &es).map_err(err)?;
        key_ids.insert(tag, (h, e, t.map_parts().expect("a map").0.bare()));
    }

    let mut ctx = module.make_context();
    let mut fctx = FunctionBuilderContext::new();
    let mut runtime = HashMap::new();
    let map_ops = define_map_ops(&mut module, &mut runtime, hir, &drop_ids, &copy_ids, &key_ids)?;
    let mut lits: Vec<Option<DataId>> = vec![None; mir.strings.len()];

    for (f, &id) in ids.iter().enumerate() {
        if hir.foreign.contains_key(&f) {
            continue;
        }
        let hf = &hir.funcs[f];
        // No body: a test block that this build leaves out (see `lib::build`).
        let Some(body) = mir.bodies[f].as_ref() else { continue };
        ctx.func.signature = module.declarations().get_function_decl(id).signature.clone();
        ctx.func.name = UserFuncName::user(0, id.as_u32());
        let cfg = module.isa().frontend_config();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let params = b.block_params(entry).to_vec();
            let blocks: Vec<ClBlock> = body.blocks.iter().map(|_| b.create_block()).collect();
            let mut g = Cg {
                b,
                module: &mut module,
                hir,
                mir,
                body,
                ids: &ids,
                runtime: &mut runtime,
                lits: &mut lits,
                ptr,
                slots: Vec::new(),
                blocks,
                ret_ptr: None,
                drop_ids: &drop_ids,
                map_ops: &map_ops,
                copy_ids: &copy_ids,
                vt_ids: &vt_ids,
                dyn_vt: None,
            };
            g.prologue(&params, hf.ret);
            let first = g.blocks[0];
            g.b.ins().jump(first, &[]);
            for bb in reverse_postorder(body) {
                g.b.switch_to_block(g.blocks[bb.index()]);
                let block = &body.blocks[bb];
                for s in &block.statements {
                    g.statement(&s.kind);
                }
                g.terminator(&block.terminator.kind);
            }
            g.b.seal_all_blocks();
            g.b.finalize(cfg);
        }
        module.define_function(id, &mut ctx).map_err(|e| format!("in `{}`: {e:?}", hf.symbol))?;
        module.clear_context(&mut ctx);
    }

    for t in drop_glue_types(hir) {
        define_glue(&mut module, &mut ctx, &mut fctx, &mut runtime, hir, &ids, &drop_ids, &copy_ids, &map_ops, t, true)?;
    }
    for t in copy_glue_types(hir) {
        define_glue(&mut module, &mut ctx, &mut fctx, &mut runtime, hir, &ids, &drop_ids, &copy_ids, &map_ops, t, false)?;
    }
    let mut tags: Vec<(String, (FuncId, FuncId, Type))> = key_ids.iter().map(|(t, f)| (t.clone(), *f)).collect();
    tags.sort_by(|a, b| a.0.cmp(&b.0));
    for (tag, (h, e, key)) in tags {
        if crate::codegen_common::key_is_composite(key) {
            define_composite_key_fns(&mut module, &mut ctx, &mut fctx, &mut runtime, hir, &ids, &drop_ids, &copy_ids, &map_ops, key, h, e)?;
        } else {
            define_key_fns(&mut module, &mut ctx, &mut fctx, &mut runtime, &tag, h, e)?;
        }
    }

    emit_c_main(&mut module, &mut ctx, &mut fctx, ids[hir.main], hir.funcs[hir.main].ret)?;
    module.finish().emit().map_err(err)
}

/// The read-only table of each (type, interface) a borrow is converted at
/// (`hir::VTable`), as words laid out as `hir::DynIface` says: sizes and
/// offsets as numbers, methods as function addresses, ancestors as the
/// addresses of their tables.
fn define_vtables(module: &mut ObjectModule, hir: &hir::Program, ids: &[FuncId]) -> Result<Vec<DataId>, String> {
    let mut data = Vec::new();
    for i in 0..hir.vtables.len() {
        data.push(module.declare_data(&format!("drachm_vt_{i}"), Linkage::Local, false, false).map_err(err)?);
    }
    for (i, vt) in hir.vtables.iter().enumerate() {
        let iface = &hir.dyn_ifaces[vt.iface];
        let (size, align) = hir.size_align(vt.ty);
        let words = 2 + iface.ancestors.len() + iface.methods.len() + iface.fields.len();
        let mut bytes = vec![0u8; words * 8];
        let mut put = |slot: usize, v: u64| bytes[slot * 8..slot * 8 + 8].copy_from_slice(&v.to_le_bytes());
        put(0, size as u64);
        put(1, align as u64);
        for (f, &off) in vt.fields.iter().enumerate() {
            put(iface.field_slot(f), off as u64);
        }
        let mut desc = DataDescription::new();
        desc.define(bytes.into_boxed_slice());
        for (m, f) in vt.methods.iter().enumerate() {
            if let Some(f) = f {
                let r = module.declare_func_in_data(ids[*f], &mut desc);
                desc.write_function_addr((iface.method_slot(m) * 8) as u32, r);
            }
        }
        for &a in &iface.ancestors {
            let r = module.declare_data_in_data(data[hir.vtable_index(vt.ty, a)], &mut desc);
            desc.write_data_addr((iface.ancestor_slot(a).expect("an ancestor") * 8) as u32, r, 0);
        }
        module.define_data(data[i], &desc).map_err(err)?;
    }
    Ok(data)
}

/// Call a runtime function, importing it on first use.
fn import_rt(
    module: &mut ObjectModule,
    runtime: &mut HashMap<&'static str, FuncId>,
    name: &'static str,
    params: &[ClType],
    returns: &[ClType],
) -> FuncId {
    if let Some(id) = runtime.get(name) {
        return *id;
    }
    let mut sig = module.make_signature();
    // C expects narrow arguments widened by the caller.
    sig.params.extend(params.iter().map(|&t| if t.bits() < 32 { AbiParam::new(t).uext() } else { AbiParam::new(t) }));
    sig.returns.extend(returns.iter().copied().map(AbiParam::new));
    let id = module.declare_function(name, Linkage::Import, &sig).expect("runtime import");
    runtime.insert(name, id);
    id
}

/// The integer pieces an enum is copied in: as wide as its alignment allows.
fn enum_chunks(hir: &hir::Program, ty: Type, at: u32, out: &mut Vec<(u32, ClType)>) {
    let (size, align) = hir.size_align(ty);
    let cl = match align {
        1 => types::I8,
        2 => types::I16,
        4 => types::I32,
        _ => types::I64,
    };
    let step = cl.bytes();
    let mut off = 0;
    while off < size {
        out.push((at + off, cl));
        off += step;
    }
}

/// Scalar leaves of a value of type `ty` that holds no strings, relative to `at`.
fn plain_leaves(hir: &hir::Program, ty: Type, at: u32, out: &mut Vec<(u32, ClType)>) {
    if ty.is_fat() || ty.is_func() {
        out.extend([(at, types::I64), (at + 8, types::I64)]);
        return;
    }
    if let Some(cl) = cl_type(ty) {
        out.push((at, cl));
        return;
    }
    if ty.enum_id().is_some() || ty.is_array() {
        enum_chunks(hir, ty, at, out);
        return;
    }
    for f in &hir.struct_def(ty).fields {
        plain_leaves(hir, f.ty, at + f.offset, out);
    }
}

/// Run `body(builder, index, item_offset)` for each of `len` items of `size`
/// bytes: a loop in the generated code, so a large array stays small.
fn each_item(b: &mut FunctionBuilder, len: u64, size: u32, body: &mut dyn FnMut(&mut FunctionBuilder, Value)) {
    let head = b.create_block();
    let work = b.create_block();
    let done = b.create_block();
    b.append_block_param(head, types::I64);
    let zero = b.ins().iconst(types::I64, 0);
    b.ins().jump(head, &[zero.into()]);
    b.switch_to_block(head);
    let i = b.block_params(head)[0];
    let more = b.ins().icmp_imm(IntCC::UnsignedLessThan, i, len as i64);
    b.ins().brif(more, work, &[], done, &[]);
    b.switch_to_block(work);
    let off = b.ins().imul_imm_u(i, size as i64);
    body(b, off);
    let next = b.ins().iadd_imm(i, 1);
    b.ins().jump(head, &[next.into()]);
    b.switch_to_block(done);
}

/// Everything a glue function needs to call others.
struct GlueEnv<'a> {
    hir: &'a hir::Program,
    ids: &'a [FuncId],
    drop_ids: &'a HashMap<Type, FuncId>,
    copy_ids: &'a HashMap<Type, FuncId>,
    map_ops: &'a HashMap<Type, DataId>,
}

/// Drop the field of type `ty` at `base + off`.
fn glue_drop_field(
    module: &mut ObjectModule,
    runtime: &mut HashMap<&'static str, FuncId>,
    env: &GlueEnv,
    b: &mut FunctionBuilder,
    base: Value,
    off: u32,
    ty: Type,
) {
    let ptr = module.target_config().pointer_type();
    let a = if off == 0 { base } else { b.ins().iadd_imm_u(base, off as i64) };
    match ty {
        Type::Str => {
            let cap = b.ins().load(types::I64, MEM, a, STR_CAP);
            let drop_b = b.create_block();
            let after = b.create_block();
            b.ins().brif(cap, drop_b, &[], after, &[]);
            b.switch_to_block(drop_b);
            let rt = import_rt(module, runtime, "drachm_str_drop", &[ptr], &[]);
            let callee = module.declare_func_in_func(rt, b.func);
            b.ins().call(callee, &[a]);
            b.ins().jump(after, &[]);
            b.switch_to_block(after);
        }
        Type::Struct { .. } | Type::Enum { .. } | Type::List { .. } | Type::Array { .. } | Type::Map { .. } if crate::mir::needs_drop(env.hir, ty) => {
            let callee = module.declare_func_in_func(env.drop_ids[&ty], b.func);
            b.ins().call(callee, &[a]);
        }
        _ => {}
    }
}

/// Deep-copy (or bit-copy) the field of type `ty` at `off` from `src` to `dst`.
fn glue_copy_field(
    module: &mut ObjectModule,
    runtime: &mut HashMap<&'static str, FuncId>,
    env: &GlueEnv,
    b: &mut FunctionBuilder,
    dst: Value,
    src: Value,
    off: u32,
    ty: Type,
) {
    let ptr = module.target_config().pointer_type();
    let at = |b: &mut FunctionBuilder, base: Value| if off == 0 { base } else { b.ins().iadd_imm_u(base, off as i64) };
    match ty {
        Type::Str => {
            let (d, s) = (at(b, dst), at(b, src));
            let rt = import_rt(module, runtime, "drachm_str_copy", &[ptr, ptr], &[]);
            let callee = module.declare_func_in_func(rt, b.func);
            b.ins().call(callee, &[d, s]);
        }
        Type::Struct { .. } | Type::Enum { .. } | Type::List { .. } | Type::Array { .. } | Type::Map { .. } if holds_string(env.hir, ty) => {
            let (d, s) = (at(b, dst), at(b, src));
            let callee = module.declare_func_in_func(env.copy_ids[&ty], b.func);
            b.ins().call(callee, &[d, s]);
        }
        t => {
            let mut leaves = Vec::new();
            plain_leaves(env.hir, t, off, &mut leaves);
            for (o, cl) in leaves {
                let v = b.ins().load(cl, MEM, src, o as i32);
                b.ins().store(MEM, v, dst, o as i32);
            }
        }
    }
}

/// Run `body` for each variant of enum `ty` for which `wanted` holds, with
/// the code for the others skipped: `if tag == v { body }` in turn.
fn for_variants(
    b: &mut FunctionBuilder,
    hir: &hir::Program,
    ty: Type,
    base: Value,
    wanted: &dyn Fn(&hir::VariantDef) -> bool,
    body: &mut dyn FnMut(&mut FunctionBuilder, usize, &hir::VariantDef),
) {
    let def = hir.enum_def(ty);
    let tag_ty = cl_type(crate::mir::ty::tag_type(def.tag_size)).unwrap();
    let tag = b.ins().load(tag_ty, MEM, base, 0);
    for (vi, v) in def.variants.iter().enumerate() {
        if !wanted(v) {
            continue;
        }
        let here = b.create_block();
        let next = b.create_block();
        let is = b.ins().icmp_imm(IntCC::Equal, tag, vi as i64);
        b.ins().brif(is, here, &[], next, &[]);
        b.switch_to_block(here);
        body(b, vi, v);
        b.ins().jump(next, &[]);
        b.switch_to_block(next);
    }
}

/// The address of the function that drops (or deep-copies) one item of type
/// `elem`, for the list runtime; null when the item is plain data.
fn glue_fn_addr(
    module: &mut ObjectModule,
    runtime: &mut HashMap<&'static str, FuncId>,
    env: &GlueEnv,
    b: &mut FunctionBuilder,
    elem: Type,
    drop: bool,
) -> Value {
    let ids = if drop { env.drop_ids } else { env.copy_ids };
    if let Some(id) = glue_fn_id(module, runtime, env.hir, ids, elem, drop) {
        let f = module.declare_func_in_func(id, b.func);
        b.ins().func_addr(module.target_config().pointer_type(), f)
    } else {
        b.ins().iconst(module.target_config().pointer_type(), 0)
    }
}

fn glue_fn_id(
    module: &mut ObjectModule,
    runtime: &mut HashMap<&'static str, FuncId>,
    hir: &hir::Program,
    ids: &HashMap<Type, FuncId>,
    elem: Type,
    drop: bool,
) -> Option<FuncId> {
    let needed = if drop { needs_drop(hir, elem) } else { holds_string(hir, elem) };
    if !needed {
        return None;
    }
    let ptr = module.target_config().pointer_type();
    Some(match (elem, drop) {
        (Type::Str, true) => import_rt(module, runtime, "drachm_str_drop", &[ptr], &[]),
        (Type::Str, false) => import_rt(module, runtime, "drachm_str_copy", &[ptr, ptr], &[]),
        _ => ids[&elem],
    })
}

/// Define the drop (`drop == true`) or copy glue function of struct or enum `ty`.
#[allow(clippy::too_many_arguments)]
fn define_glue(
    module: &mut ObjectModule,
    ctx: &mut cranelift_codegen::Context,
    fctx: &mut FunctionBuilderContext,
    runtime: &mut HashMap<&'static str, FuncId>,
    hir: &hir::Program,
    ids: &[FuncId],
    drop_ids: &HashMap<Type, FuncId>,
    copy_ids: &HashMap<Type, FuncId>,
    map_ops: &HashMap<Type, DataId>,
    ty: Type,
    drop: bool,
) -> Result<(), String> {
    let id = if drop { drop_ids[&ty] } else { copy_ids[&ty] };
    let ptr = module.target_config().pointer_type();
    ctx.func.signature = module.declarations().get_function_decl(id).signature.clone();
    ctx.func.name = UserFuncName::user(0, id.as_u32());
    let cfg = module.isa().frontend_config();
    let env = GlueEnv { hir, ids, drop_ids, copy_ids, map_ops };
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let params = b.block_params(entry).to_vec();
        if drop {
            let p = params[0];
            match ty {
                Type::Map { .. } => {
                    let ops = map_ops_addr(module, &mut b, env.map_ops, ty);
                    let rt = import_rt(module, runtime, "drachm_map_drop", &[ptr, ptr], &[]);
                    let callee = module.declare_func_in_func(rt, b.func);
                    b.ins().call(callee, &[p, ops]);
                }
                Type::List { elem } => {
                    let (size, _) = hir.size_align(*elem);
                    let size = b.ins().iconst(types::I64, size as i64);
                    let f = glue_fn_addr(module, runtime, &env, &mut b, *elem, true);
                    let rt = import_rt(module, runtime, "drachm_list_drop", &[ptr, types::I64, ptr], &[]);
                    let callee = module.declare_func_in_func(rt, b.func);
                    b.ins().call(callee, &[p, size, f]);
                }
                Type::Array { elem, len } => {
                    let (size, _) = hir.size_align(*elem);
                    each_item(&mut b, len, size, &mut |b, off| {
                        let at = b.ins().iadd(p, off);
                        glue_drop_field(module, runtime, &env, b, at, 0, *elem);
                    });
                }
                Type::Struct { id: sid, .. } => {
                    let def = &hir.structs[sid];
                    if let Some(f) = def.has_drop {
                        let callee = module.declare_func_in_func(ids[f], b.func);
                        b.ins().call(callee, &[p]);
                    }
                    for f in &def.fields {
                        glue_drop_field(module, runtime, &env, &mut b, p, f.offset, f.ty);
                    }
                }
                _ => {
                    let owning = |v: &hir::VariantDef| v.fields.iter().any(|f| crate::mir::needs_drop(hir, f.ty));
                    for_variants(&mut b, hir, ty, p, &owning, &mut |b, _, v| {
                        for f in &v.fields {
                            glue_drop_field(module, runtime, &env, b, p, f.offset, f.ty);
                        }
                    });
                    let def = hir.enum_def(ty);
                    if def.kind == hir::EnumKind::Failure {
                        // The cause word is at the same offset in every variant.
                        let word = b.ins().load(types::I64, MEM, p, def.variants[0].fields[0].offset as i32);
                        let rt = import_rt(module, runtime, "drachm_cause_drop", &[types::I64], &[]);
                        let callee = module.declare_func_in_func(rt, b.func);
                        b.ins().call(callee, &[word]);
                    }
                }
            }
        } else {
            let (dst, src) = (params[0], params[1]);
            match ty {
                Type::Map { .. } => {
                    let ops = map_ops_addr(module, &mut b, env.map_ops, ty);
                    let rt = import_rt(module, runtime, "drachm_map_copy", &[ptr, ptr, ptr], &[]);
                    let callee = module.declare_func_in_func(rt, b.func);
                    b.ins().call(callee, &[dst, src, ops]);
                }
                Type::List { elem } => {
                    let (size, align) = hir.size_align(*elem);
                    let (size, align) = (b.ins().iconst(types::I64, size as i64), b.ins().iconst(types::I64, align as i64));
                    let f = glue_fn_addr(module, runtime, &env, &mut b, *elem, false);
                    let rt = import_rt(module, runtime, "drachm_list_copy", &[ptr, ptr, types::I64, types::I64, ptr], &[]);
                    let callee = module.declare_func_in_func(rt, b.func);
                    b.ins().call(callee, &[dst, src, size, align, f]);
                }
                Type::Array { elem, len } => {
                    let (size, _) = hir.size_align(*elem);
                    each_item(&mut b, len, size, &mut |b, off| {
                        let (d, s) = (b.ins().iadd(dst, off), b.ins().iadd(src, off));
                        glue_copy_field(module, runtime, &env, b, d, s, 0, *elem);
                    });
                }
                Type::Struct { id: sid, .. } => {
                    for f in &hir.structs[sid].fields {
                        glue_copy_field(module, runtime, &env, &mut b, dst, src, f.offset, f.ty);
                    }
                }
                _ => {
                    // Every bit first, then the owning fields of the variant the tag names, deeply.
                    let mut chunks = Vec::new();
                    enum_chunks(hir, ty, 0, &mut chunks);
                    for (off, cl) in chunks {
                        let v = b.ins().load(cl, MEM, src, off as i32);
                        b.ins().store(MEM, v, dst, off as i32);
                    }
                    let owning = |v: &hir::VariantDef| v.fields.iter().any(|f| holds_string(hir, f.ty));
                    for_variants(&mut b, hir, ty, src, &owning, &mut |b, _, v| {
                        for f in &v.fields {
                            if holds_string(hir, f.ty) {
                                glue_copy_field(module, runtime, &env, b, dst, src, f.offset, f.ty);
                            }
                        }
                    });
                }
            }
        }
        b.ins().return_(&[]);
        b.seal_all_blocks();
        b.finalize(cfg);
    }
    module.define_function(id, ctx).map_err(|e| format!("in the glue of `{ty}`: {e:?}"))?;
    module.clear_context(ctx);
    Ok(())
}

/// Where a local lives.
#[derive(Clone, Copy)]
enum Slot {
    /// Type `none`.
    Nothing,
    Var(Variable),
    Mem(Value),
}

/// Where a place is: an SSA variable, or memory at `base + offset`.
#[derive(Clone, Copy)]
enum Loc {
    Var(Variable),
    Mem(Value, u32),
}

#[derive(Clone, Copy)]
enum Leaf {
    Scalar(ClType),
    Str,
    /// This many bytes, copied with `memcpy`.
    Bulk(u32),
}

struct Cg<'a, 'b> {
    b: FunctionBuilder<'b>,
    module: &'a mut ObjectModule,
    hir: &'a hir::Program,
    mir: &'a Program,
    body: &'a Body,
    ids: &'a [FuncId],
    runtime: &'a mut HashMap<&'static str, FuncId>,
    /// The read-only data of each string literal emitted so far, by `StrId`.
    lits: &'a mut Vec<Option<DataId>>,
    ptr: ClType,
    slots: Vec<Slot>,
    blocks: Vec<ClBlock>,
    /// Where a function that returns a string or struct writes it.
    ret_ptr: Option<Value>,
    /// The drop and copy glue functions, by `StructId`.
    drop_ids: &'a HashMap<Type, FuncId>,
    copy_ids: &'a HashMap<Type, FuncId>,
    /// The data of each vtable, by index in `hir::Program::vtables`.
    vt_ids: &'a [DataId],
    /// The table word of the fat borrow the last `Deref` went through.
    dyn_vt: Option<Value>,
    /// The hash and equality functions of each map key tag (S6D).
    map_ops: &'a HashMap<Type, DataId>,
}

impl Cg<'_, '_> {
    // ---- locals ------------------------------------------------------------

    fn prologue(&mut self, params: &[Value], ret: Type) {
        let hidden = in_memory(ret) as usize;
        self.ret_ptr = (hidden == 1).then(|| params[0]);
        let body = self.body;
        for (l, d) in body.locals.iter_enumerated() {
            let i = l.index();
            let ty = d.ty;
            let incoming = (1..=body.arg_count).contains(&i).then(|| params[hidden + i - 1]);
            let slot = if i == 0 {
                match (self.ret_ptr, cl_type(ty)) {
                    (Some(p), _) => Slot::Mem(p),
                    (None, Some(_)) if d.in_memory => Slot::Mem(self.mem(ty)),
                    (None, Some(cl)) => Slot::Var(self.b.declare_var(cl)),
                    (None, None) => Slot::Nothing,
                }
            } else {
                match cl_type(ty) {
                    Some(_) if d.in_memory => {
                        let addr = self.mem(ty);
                        if let Some(v) = incoming {
                            self.b.ins().store(MEM, v, addr, 0);
                        }
                        Slot::Mem(addr)
                    }
                    Some(cl) => {
                        let var = self.b.declare_var(cl);
                        if let Some(v) = incoming {
                            self.b.def_var(var, v);
                        }
                        Slot::Var(var)
                    }
                    // A string parameter is taken over from the caller.
                    None if ty == Type::Str => {
                        let addr = self.mem(ty);
                        match incoming {
                            Some(from) => self.copy_bits(ty, addr, from),
                            None => self.clear(addr),
                        }
                        Slot::Mem(addr)
                    }
                    // A struct parameter's memory is the callee's own.
                    None if in_memory(ty) => match incoming {
                        Some(p) => Slot::Mem(p),
                        None => Slot::Mem(self.mem(ty)),
                    },
                    None => Slot::Nothing,
                }
            };
            self.slots.push(slot);
        }
    }

    /// Fresh memory for a value of type `ty`; returns its address.
    fn mem(&mut self, ty: Type) -> Value {
        let (size, align) = self.hir.size_align(ty);
        let slot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, size.max(1), align.trailing_zeros() as u8));
        self.b.ins().stack_addr(self.ptr, slot, 0)
    }

    /// `base + off`.
    fn offset(&mut self, base: Value, off: u32) -> Value {
        if off == 0 { base } else { self.b.ins().iadd_imm_u(base, off as i64) }
    }

    /// Where `p` is, and its type. A `Deref` loads the pointer it goes through.
    fn locate(&mut self, p: &Place) -> (Loc, Type) {
        let mut ty = self.body.locals[p.local].ty;
        let mut loc = match self.slots[p.local.index()] {
            Slot::Var(v) => Loc::Var(v),
            Slot::Mem(a) => Loc::Mem(a, 0),
            Slot::Nothing => unreachable!("{} has no storage", p.local),
        };
        let mut variant = None;
        for pr in &p.proj {
            match *pr {
                Proj::Downcast(v) => variant = Some(v),
                Proj::Field(i) if matches!(ty, Type::Dyn { .. }) => {
                    // A required field: its offset is in the table.
                    let Type::Dyn { id, .. } = ty else { unreachable!() };
                    let Loc::Mem(base, off) = loc else { unreachable!("behind a borrow") };
                    let iface = &self.hir.dyn_ifaces[id];
                    let (slot, fty) = (iface.field_slot(i), iface.fields[i].ty);
                    let vt = self.dyn_vt.expect("a table behind the borrow");
                    let word = self.b.ins().load(self.ptr, MEM, vt, (slot * 8) as i32);
                    let at = self.b.ins().iadd(base, word);
                    loc = Loc::Mem(at, off);
                    ty = fty;
                }
                Proj::Field(i) => {
                    let Loc::Mem(base, off) = loc else { unreachable!("a field of a register") };
                    let f = match variant.take() {
                        Some(v) => &self.hir.enum_def(ty).variants[v].fields[i],
                        None => &self.hir.struct_def(ty).fields[i],
                    };
                    loc = Loc::Mem(base, off + f.offset);
                    ty = f.ty;
                }
                Proj::Deref if ty.is_fat() => {
                    // The borrow is two words in memory: the address, then the table.
                    let Loc::Mem(base, off) = loc else { unreachable!("a fat borrow is in memory") };
                    let data = self.b.ins().load(self.ptr, MEM, base, off as i32);
                    self.dyn_vt = Some(self.b.ins().load(self.ptr, MEM, base, off as i32 + 8));
                    loc = Loc::Mem(data, 0);
                    ty = ty.target().expect("a borrow");
                }
                Proj::Deref => {
                    let ptr = match loc {
                        Loc::Var(v) => self.b.use_var(v),
                        Loc::Mem(base, off) => self.b.ins().load(self.ptr, MEM, base, off as i32),
                    };
                    loc = Loc::Mem(ptr, 0);
                    ty = ty.target().expect("deref of a non-pointer");
                }
                Proj::Index(il) => {
                    let Loc::Mem(base, off) = loc else { unreachable!("a list or array is in memory") };
                    let i = self.operand(&Operand::Copy(Place::local(il)));
                    let elem = ty.elem().expect("an item of a list or array");
                    let (size, _) = self.hir.size_align(elem);
                    let data = if ty.is_list() { self.b.ins().load(self.ptr, MEM, base, off as i32) } else { self.offset(base, off) };
                    let scaled = if size == 1 { i } else { self.b.ins().imul_imm_u(i, size as i64) };
                    let addr = self.b.ins().iadd(data, scaled);
                    loc = Loc::Mem(addr, 0);
                    ty = elem;
                }
            }
        }
        (loc, ty)
    }

    /// The address of a place that is in memory.
    fn addr_of(&mut self, p: &Place) -> Value {
        match self.locate(p).0 {
            Loc::Mem(base, off) => self.offset(base, off),
            Loc::Var(_) => unreachable!("{} is borrowed but not in memory", p.local),
        }
    }

    fn load(&mut self, loc: Loc, ty: Type) -> Value {
        match loc {
            Loc::Var(v) => self.b.use_var(v),
            Loc::Mem(base, off) => self.b.ins().load(cl_type(ty).expect("a scalar"), MEM, base, off as i32),
        }
    }

    fn store(&mut self, loc: Loc, v: Value) {
        match loc {
            Loc::Var(var) => self.b.def_var(var, v),
            Loc::Mem(base, off) => {
                self.b.ins().store(MEM, v, base, off as i32);
            }
        }
    }

    // ---- operands ----------------------------------------------------------

    fn op_ty(&self, op: &Operand) -> Type {
        operand_ty(self.body, self.hir, op).expect("validated MIR")
    }

    /// The value of a scalar operand.
    fn operand(&mut self, op: &Operand) -> Value {
        match op {
            Operand::Copy(p) | Operand::Move(p) => {
                let (loc, ty) = self.locate(p);
                self.load(loc, ty)
            }
            Operand::Const(Constant::Int(bits, ty)) => self.int_const(cl_type(*ty).expect("an integer type"), *bits),
            Operand::Const(Constant::Float(v, Type::Float(32))) => self.b.ins().f32const(*v as f32),
            Operand::Const(Constant::Float(v, _)) => self.b.ins().f64const(*v),
            Operand::Const(Constant::Bool(v)) => self.b.ins().iconst(types::I8, *v as i64),
            Operand::Const(Constant::FuncAddr(f, _)) => {
                let fref = self.module.declare_func_in_func(self.ids[*f], self.b.func);
                self.b.ins().func_addr(self.ptr, fref)
            }
            Operand::Const(Constant::Str(_)) => unreachable!("a string constant is not a scalar"),
        }
    }

    /// The address of a string or struct operand, without copying it (a
    /// string constant gets a temporary).
    fn operand_addr(&mut self, op: &Operand) -> Value {
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.addr_of(p),
            Operand::Const(Constant::Str(id)) => {
                let addr = self.mem(Type::Str);
                self.store_literal(addr, *id);
                addr
            }
            Operand::Const(_) => unreachable!("not a string or struct"),
        }
    }

    // ---- values in memory --------------------------------------------------

    fn leaves(&self, ty: Type, at: u32, out: &mut Vec<(u32, Leaf)>) {
        if ty.is_fat() || ty.is_func() {
            out.extend([(at, Leaf::Scalar(types::I64)), (at + 8, Leaf::Scalar(types::I64))]);
            return;
        }
        if ty == Type::Str {
            out.push((at, Leaf::Str));
            return;
        }
        if let Some(cl) = cl_type(ty) {
            out.push((at, Leaf::Scalar(cl)));
            return;
        }
        if ty.is_list() {
            out.push((at, Leaf::Str));
            return;
        }
        if ty.is_map() {
            // The runtime's header: six words (`drachm_map`).
            out.extend((0..6).map(|w| (at + 8 * w, Leaf::Scalar(types::I64))));
            return;
        }
        if let Type::Array { .. } = ty {
            let (size, _) = self.hir.size_align(ty);
            if size > 64 {
                out.push((at, Leaf::Bulk(size)));
                return;
            }
        }
        if ty.enum_id().is_some() || ty.is_array() {
            let mut chunks = Vec::new();
            enum_chunks(self.hir, ty, at, &mut chunks);
            out.extend(chunks.into_iter().map(|(o, cl)| (o, Leaf::Scalar(cl))));
            return;
        }
        for f in &self.hir.struct_def(ty).fields {
            self.leaves(f.ty, at + f.offset, out);
        }
    }

    /// Copy the bits of a value of type `ty` (padding is never read).
    fn copy_bits(&mut self, ty: Type, dst: Value, src: Value) {
        let mut leaves = Vec::new();
        self.leaves(ty, 0, &mut leaves);
        for (off, leaf) in leaves {
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
                Leaf::Bulk(size) => {
                    let (d, s) = (self.offset(dst, off as u32), self.offset(src, off as u32));
                    let n = self.b.ins().iconst(self.ptr, size as i64);
                    let cfg = self.module.target_config();
                    self.b.call_memcpy(cfg, d, s, n);
                }
            }
        }
    }

    /// Make the string at `addr` empty, without freeing.
    fn clear(&mut self, addr: Value) {
        let zero = self.b.ins().iconst(types::I64, 0);
        for word in [0, STR_LEN, STR_CAP] {
            self.b.ins().store(MEM, zero, addr, word);
        }
    }

    fn store_str(&mut self, dest: Value, ptr: Value, len: Value, cap: Value) {
        self.b.ins().store(MEM, ptr, dest, 0);
        self.b.ins().store(MEM, len, dest, STR_LEN);
        self.b.ins().store(MEM, cap, dest, STR_CAP);
    }

    /// Write a not-owned view of a literal to `dest`.
    fn store_literal(&mut self, dest: Value, id: StrId) {
        let (p, len) = self.literal(id);
        let zero = self.b.ins().iconst(types::I64, 0);
        self.store_str(dest, p, len, zero);
    }

    fn literal(&mut self, id: StrId) -> (Value, Value) {
        let s = &self.mir.strings[id];
        let len = self.b.ins().iconst(types::I64, s.len() as i64);
        if s.is_empty() {
            let null = self.b.ins().iconst(self.ptr, 0);
            return (null, len);
        }
        let data = match self.lits[id.index()] {
            Some(d) => d,
            None => {
                let name = format!("drachm_lit_{}", id.index());
                let d = self.module.declare_data(&name, Linkage::Local, false, false).expect("string data");
                let mut desc = DataDescription::new();
                desc.define(s.as_bytes().to_vec().into_boxed_slice());
                self.module.define_data(d, &desc).expect("string data");
                self.lits[id.index()] = Some(d);
                d
            }
        };
        let gv = self.module.declare_data_in_func(data, self.b.func);
        (self.b.ins().symbol_value(self.ptr, gv), len)
    }

    // ---- statements --------------------------------------------------------

    fn statement(&mut self, k: &StatementKind) {
        match k {
            StatementKind::Assign(dest, rv) => self.assign(dest, rv),
            StatementKind::Drop { place, flag, .. } => {
                let ty = crate::mir::ty::place_ty(self.body, self.hir, place).expect("validated MIR");
                if !needs_drop(self.hir, ty) {
                    return;
                }
                // With a flag, only when it is set.
                let after = self.b.create_block();
                if let Some(f) = flag {
                    let c = self.operand(&Operand::Copy(Place::local(*f)));
                    let then = self.b.create_block();
                    self.b.ins().brif(c, then, &[], after, &[]);
                    self.b.switch_to_block(then);
                }
                let (loc, _) = self.locate(place);
                let Loc::Mem(base, off) = loc else { unreachable!("a string or struct is in memory") };
                let a = self.offset(base, off);
                match ty {
                    Type::Str => {
                        // `drachm_str_drop` starts with this test; doing it here skips
                        // the call for literals and empty strings.
                        let cap = self.b.ins().load(types::I64, MEM, a, STR_CAP);
                        let drop_b = self.b.create_block();
                        self.b.ins().brif(cap, drop_b, &[], after, &[]);
                        self.b.switch_to_block(drop_b);
                        self.rt("drachm_str_drop", &[self.ptr], &[], &[a]);
                        self.b.ins().jump(after, &[]);
                    }
                    _ => {
                        let id = self.drop_ids[&ty];
                        let callee = self.module.declare_func_in_func(id, self.b.func);
                        self.b.ins().call(callee, &[a]);
                        self.b.ins().jump(after, &[]);
                    }
                }
                self.b.switch_to_block(after);
            }
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::FakeRead(_) | StatementKind::Nop => {}
        }
    }

    fn assign(&mut self, dest: &Place, rv: &Rvalue) {
        let ty = crate::mir::ty::place_ty(self.body, self.hir, dest).expect("validated MIR");
        if ty == Type::None {
            return;
        }
        if cl_type(ty).is_some() {
            let v = self.rvalue(rv);
            let (loc, _) = self.locate(dest);
            self.store(loc, v);
            return;
        }
        match rv {
            Rvalue::Use(Operand::Const(Constant::Str(id))) => {
                let (loc, _) = self.locate(dest);
                let Loc::Mem(base, off) = loc else { unreachable!() };
                let a = self.offset(base, off);
                self.store_literal(a, *id);
            }
            Rvalue::Use(op @ (Operand::Copy(_) | Operand::Move(_))) => {
                let src = self.operand_addr(op);
                let d = self.addr_of(dest);
                self.copy_bits(ty, d, src);
            }
            // A reborrow of the interface behind a fat borrow is the fat borrow.
            Rvalue::Ref(_, p) => {
                let mut q = p.clone();
                assert_eq!(q.proj.pop(), Some(Proj::Deref), "a place of interface type is behind a borrow");
                let src = self.addr_of(&q);
                let d = self.addr_of(dest);
                self.copy_bits(ty, d, src);
            }
            Rvalue::MakeFn(id, _, env) => {
                let f = self.module.declare_func_in_func(self.ids[*id], self.b.func);
                let code = self.b.ins().func_addr(self.ptr, f);
                let env = match env {
                    Some(p) => self.addr_of(p),
                    None => self.b.ins().iconst(self.ptr, 0),
                };
                let d = self.addr_of(dest);
                self.b.ins().store(MEM, code, d, 0);
                self.b.ins().store(MEM, env, d, 8);
            }
            Rvalue::Unsize(op, to) => {
                let from = self.op_ty(op);
                let Some(Type::Dyn { id: k, .. }) = to.target() else { unreachable!("unsized to an interface") };
                let (data, vt) = match from.target() {
                    // An upcast: the table of the same type for the ancestor.
                    Some(Type::Dyn { id: j, .. }) => {
                        let src = self.operand_addr(op);
                        let slot = self.hir.dyn_ifaces[j].ancestor_slot(k).expect("an ancestor");
                        let data = self.b.ins().load(self.ptr, MEM, src, 0);
                        let old = self.b.ins().load(self.ptr, MEM, src, 8);
                        (data, self.b.ins().load(self.ptr, MEM, old, (slot * 8) as i32))
                    }
                    _ => {
                        let data = self.operand(op);
                        let table = self.hir.vtable_index(from.target().expect("a borrow"), k);
                        let gv = self.module.declare_data_in_func(self.vt_ids[table], self.b.func);
                        (data, self.b.ins().symbol_value(self.ptr, gv))
                    }
                };
                let d = self.addr_of(dest);
                self.b.ins().store(MEM, data, d, 0);
                self.b.ins().store(MEM, vt, d, 8);
            }
            Rvalue::Aggregate(sid, ops) if self.hir.structs[*sid].is_union => {
                // A union starts as zero bytes.
                let d = self.addr_of(dest);
                let zero = self.b.ins().iconst(types::I8, 0);
                let n = self.b.ins().iconst(self.ptr, self.hir.structs[*sid].size as i64);
                let cfg = self.module.target_config();
                self.b.call_memset(cfg, d, zero, n);
            }
            Rvalue::Aggregate(sid, ops) => {
                let offs: Vec<u32> = self.hir.structs[*sid].fields.iter().map(|f| f.offset).collect();
                self.aggregate(dest, ty, &offs, ops, None)
            }
            Rvalue::Array(t, ops) => {
                let (size, _) = self.hir.size_align(t.elem().expect("an array"));
                let offs: Vec<u32> = (0..ops.len() as u32).map(|i| i * size).collect();
                self.aggregate(dest, ty, &offs, ops, None)
            }
            Rvalue::MapNew(_) => {
                let d = self.addr_of(dest);
                self.rt("drachm_map_new", &[self.ptr], &[], &[d]);
            }
            Rvalue::ListNew(t, cap) => {
                let d = self.addr_of(dest);
                if *cap == 0 {
                    self.clear(d);
                } else {
                    let (size, align) = self.hir.size_align(t.elem().expect("a list"));
                    let (c, s, a) = (self.b.ins().iconst(types::I64, *cap as i64), self.b.ins().iconst(types::I64, size as i64), self.b.ins().iconst(types::I64, align as i64));
                    self.rt("drachm_list_with_cap", &[self.ptr, types::I64, types::I64, types::I64], &[], &[d, c, s, a]);
                }
            }
            Rvalue::Variant(eid, vi, ops) => {
                let def = &self.hir.enums[*eid];
                let offs: Vec<u32> = def.variants[*vi].fields.iter().map(|f| f.offset).collect();
                let tag_ty = cl_type(crate::mir::ty::tag_type(def.tag_size)).unwrap();
                self.aggregate(dest, ty, &offs, ops, Some((tag_ty, *vi as i64)))
            }
            Rvalue::DeepCopy(src) => {
                let s = self.addr_of(src);
                let d = self.addr_of(dest);
                match ty {
                    Type::Str => {
                        self.rt("drachm_str_copy", &[self.ptr, self.ptr], &[], &[d, s]);
                    }
                    Type::Struct { .. } | Type::Enum { .. } | Type::List { .. } | Type::Array { .. } | Type::Map { .. } if holds_string(self.hir, ty) => {
                        let callee = self.module.declare_func_in_func(self.copy_ids[&ty], self.b.func);
                        self.b.ins().call(callee, &[d, s]);
                    }
                    _ => self.copy_bits(ty, d, s),
                }
            }
            _ => unreachable!("not a string or struct rvalue: {rv:?}"),
        }
    }

    fn aggregate(&mut self, dest: &Place, ty: Type, offsets: &[u32], ops: &[Operand], tag: Option<(ClType, i64)>) {
        // Read every scalar operand before writing anything: an operand may
        // be part of the destination (`p = P(p.y, p.x)`).
        enum Part {
            Scalar(Value),
            Mem(Type, Value),
        }
        let mut parts = Vec::new();
        let mut overlaps = false;
        for op in ops {
            let t = self.op_ty(op);
            if cl_type(t).is_some() {
                parts.push(Part::Scalar(self.operand(op)));
            } else {
                if matches!(op, Operand::Copy(p) | Operand::Move(p) if p.local == dest.local) {
                    overlaps = true;
                }
                parts.push(Part::Mem(t, self.operand_addr(op)));
            }
        }
        let target = if overlaps { self.mem(ty) } else { self.addr_of(dest) };
        if let Some((tag_ty, v)) = tag {
            let t = self.b.ins().iconst(tag_ty, v);
            self.b.ins().store(MEM, t, target, 0);
        }
        for (i, part) in parts.into_iter().enumerate() {
            let off = offsets[i];
            match part {
                Part::Scalar(v) => {
                    self.b.ins().store(MEM, v, target, off as i32);
                }
                Part::Mem(t, src) => {
                    let at = self.offset(target, off);
                    self.copy_bits(t, at, src);
                }
            }
        }
        if overlaps {
            let d = self.addr_of(dest);
            self.copy_bits(ty, d, target);
        }
    }

    /// The value of a scalar rvalue.
    fn rvalue(&mut self, rv: &Rvalue) -> Value {
        match rv {
            Rvalue::Use(op) => self.operand(op),
            Rvalue::Neg(op) => {
                let ty = self.op_ty(op);
                let v = self.operand(op);
                if ty.is_float() { self.b.ins().fneg(v) } else { self.b.ins().ineg(v) }
            }
            Rvalue::Not(op) => {
                let v = self.operand(op);
                self.b.ins().bxor_imm_u(v, 1)
            }
            Rvalue::BitNot(op) => {
                let v = self.operand(op);
                self.b.ins().bnot(v)
            }
            Rvalue::Binary(op, l, r) => {
                let operand = self.op_ty(l);
                let a = self.operand(l);
                let b = self.operand(r);
                self.binary(*op, operand, a, b)
            }
            Rvalue::Shift { left, value, amount } => {
                let ty = self.op_ty(value);
                let v = self.operand(value);
                let n = self.operand(amount);
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
            Rvalue::IntPow(base, exp) => {
                let ty = self.op_ty(base);
                let exp_ty = self.op_ty(exp);
                let cl = cl_type(ty).unwrap();
                let bv = self.operand(base);
                let xv = self.operand(exp);
                let exp_cl = self.b.func.dfg.value_type(xv);
                let negative = if is_signed(exp_ty) {
                    let zero = self.zero(exp_cl);
                    let lt = self.b.ins().icmp(IntCC::SignedLessThan, xv, zero);
                    self.b.ins().uextend(types::I32, lt)
                } else {
                    self.b.ins().iconst(types::I32, 0)
                };
                let base_signed = self.b.ins().iconst(types::I32, is_signed(ty) as i64);
                let (i32t, i64t) = (types::I32, types::I64);
                if cl.bits() <= 64 && exp_cl.bits() <= 64 {
                    let b64 = self.widen64(bv, ty);
                    let x64 = self.widen64(xv, exp_ty);
                    let r = self.rt("drachm_ipow64", &[i64t, i64t, i32t, i32t], &[i64t], &[b64, x64, negative, base_signed]);
                    self.join_halves(&[r[0], r[0]], cl)
                } else {
                    let [blo, bhi] = self.halves(bv, ty);
                    let [xlo, xhi] = self.halves(xv, exp_ty);
                    let r = self.rt(
                        "drachm_ipow128",
                        &[i64t, i64t, i64t, i64t, i32t, i32t],
                        &[i64t, i64t],
                        &[blo, bhi, xlo, xhi, negative, base_signed],
                    );
                    self.join_halves(&r, cl)
                }
            }
            Rvalue::Convert(op, to) => {
                let from = self.op_ty(op);
                let v = self.operand(op);
                self.convert(v, from, *to)
            }
            Rvalue::Checked(op, mode, a, b) => self.checked(*op, *mode, a, b),
            Rvalue::Discriminant(p) => {
                let (loc, ty) = self.locate(p);
                let Loc::Mem(base, off) = loc else { unreachable!("an enum is in memory") };
                let tag_ty = cl_type(crate::mir::ty::tag_type(self.hir.enum_def(ty).tag_size)).unwrap();
                self.b.ins().load(tag_ty, MEM, base, off as i32)
            }
            Rvalue::Len(p) => {
                let (loc, ty) = self.locate(p);
                match ty {
                    Type::Array { len, .. } => self.b.ins().iconst(types::I64, len as i64),
                    _ => {
                        let Loc::Mem(base, off) = loc else { unreachable!("a list is in memory") };
                        self.b.ins().load(types::I64, MEM, base, off as i32 + STR_LEN)
                    }
                }
            }
            Rvalue::Ref(_, p) | Rvalue::RawRef(p) => self.addr_of(p),
            // `copy p` of a raw pointer: the bits.
            Rvalue::DeepCopy(p) if self.op_ty(&Operand::Copy(p.clone())).is_ptr() => self.operand(&Operand::Copy(p.clone())),
            Rvalue::DeepCopy(_) | Rvalue::Aggregate(..) | Rvalue::Variant(..) | Rvalue::Array(..) | Rvalue::ListNew(..) | Rvalue::Unsize(..) | Rvalue::MakeFn(..) | Rvalue::MapNew(_) => unreachable!("not a scalar rvalue: {rv:?}"),
        }
    }

    // ---- terminators -------------------------------------------------------

    fn terminator(&mut self, k: &TerminatorKind) {
        match k {
            TerminatorKind::Goto(t) => {
                self.b.ins().jump(self.blocks[t.index()], &[]);
            }
            TerminatorKind::If { cond, then, els } => {
                let c = self.operand(cond);
                self.b.ins().brif(c, self.blocks[then.index()], &[], self.blocks[els.index()], &[]);
            }
            TerminatorKind::Return => {
                if self.ret_ptr.is_some() || matches!(self.slots[0], Slot::Nothing) {
                    self.b.ins().return_(&[]);
                } else {
                    let ty = self.body.locals[Local(0)].ty;
                    let v = match self.slots[0] {
                        Slot::Var(var) => self.b.use_var(var),
                        Slot::Mem(a) => self.load(Loc::Mem(a, 0), ty),
                        Slot::Nothing => unreachable!(),
                    };
                    self.b.ins().return_(&[v]);
                }
            }
            TerminatorKind::Unreachable => {
                self.b.ins().trap(TrapCode::unwrap_user(1));
            }
            TerminatorKind::Call { callee, args, dest, target } => {
                self.call(*callee, args, dest.as_ref());
                self.b.ins().jump(self.blocks[target.index()], &[]);
            }
            TerminatorKind::Switch { discr, cases, otherwise } => {
                let v = self.operand(discr);
                let mut sw = Switch::new();
                for (val, t) in cases {
                    sw.set_entry(*val, self.blocks[t.index()]);
                }
                sw.emit(&mut self.b, v, self.blocks[otherwise.index()]);
            }
            // `PanicKind` has no values.
            TerminatorKind::Panic(_) => unreachable!("the builder never emits {k:?}"),
        }
    }

    /// The address a string result is written to.
    fn dest_addr(&mut self, dest: Option<&Place>) -> Value {
        self.addr_of(dest.expect("a call with a string result has a destination"))
    }

    /// A call of a C function. Small structs travel in registers, one value
    /// per eightbyte, as the System V x86-64 ABI says (the front end refuses
    /// structs by value on other targets).
    fn foreign_call(&mut self, id: usize, args: &[Arg], dest: Option<&Place>) {
        let ret = self.hir.funcs[id].ret;
        let mut vals = Vec::new();
        for a in args {
            let ty = self.op_ty(&a.op);
            if ty.struct_id().is_none() {
                vals.push(self.operand(&a.op));
                continue;
            }
            let parts = sysv_struct(self.hir, ty).expect("a C struct passed by value fits two registers");
            let addr = self.operand_addr(&a.op);
            for (i, part) in parts.iter().enumerate() {
                let off = (8 * i) as i32;
                let v = match *part {
                    Eightbyte::Sse(_) => self.b.ins().load(part.reg_type(), MEM, addr, off),
                    Eightbyte::Int(8) => self.b.ins().load(types::I64, MEM, addr, off),
                    Eightbyte::Int(n) => {
                        let narrow = self.b.ins().load(ClType::int((n * 8) as u16).expect("a 1, 2 or 4 byte tail"), MEM, addr, off);
                        self.b.ins().uextend(types::I64, narrow)
                    }
                };
                vals.push(v);
            }
        }
        let callee = self.module.declare_func_in_func(self.ids[id], self.b.func);
        let call = self.b.ins().call(callee, &vals);
        let Some(d) = dest else { return };
        if ret.struct_id().is_some() {
            let parts = sysv_struct(self.hir, ret).expect("a C struct returned by value fits two registers");
            let base = self.dest_addr(Some(d));
            let results = self.b.inst_results(call).to_vec();
            for (i, (part, r)) in parts.iter().zip(results).enumerate() {
                let v = match *part {
                    Eightbyte::Int(n) if n < 8 => self.b.ins().ireduce(ClType::int((n * 8) as u16).expect("a 1, 2 or 4 byte tail"), r),
                    _ => r,
                };
                self.b.ins().store(MEM, v, base, (8 * i) as i32);
            }
        } else if cl_type(ret).is_some() {
            let r = self.b.inst_results(call)[0];
            let (loc, _) = self.locate(d);
            self.store(loc, r);
        }
    }

    /// The values a Drachm function is called with: scalars as they are, strings
    /// and structs by address (the callee owns them, see the module comment).
    fn call_args(&mut self, args: &[Arg]) -> Vec<Value> {
        let mut vals = Vec::new();
        for a in args {
            let ty = self.op_ty(&a.op);
            let v = if !in_memory(ty) {
                self.operand(&a.op)
            } else {
                match &a.op {
                    // The source is dead afterwards: the callee may use it as its own.
                    Operand::Move(p) => {
                        assert!(!p.proj.contains(&Proj::Deref), "the callee would free memory a borrow owns (E0310 comes first)");
                        self.addr_of(p)
                    }
                    Operand::Copy(_) | Operand::Const(_) => {
                        let src = self.operand_addr(&a.op);
                        let tmp = self.mem(ty);
                        self.copy_bits(ty, tmp, src);
                        tmp
                    }
                }
            };
            vals.push(v);
        }
        vals
    }

    fn call(&mut self, callee: Callee, args: &[Arg], dest: Option<&Place>) {
        let (i32t, i64t, ptr) = (types::I32, types::I64, self.ptr);
        match callee {
            Callee::Dyn { .. } | Callee::Fn(_) => {
                let fat = self.operand_addr(&args[0].op);
                let (ret, params, data, code) = match callee {
                    Callee::Dyn { iface, method } => {
                        let iface = &self.hir.dyn_ifaces[iface];
                        let m = &iface.methods[method];
                        let data = self.b.ins().load(ptr, MEM, fat, 0);
                        let vt = self.b.ins().load(ptr, MEM, fat, 8);
                        let code = self.b.ins().load(ptr, MEM, vt, (iface.method_slot(method) * 8) as i32);
                        (m.ret, m.params.as_slice(), data, code)
                    }
                    Callee::Fn(Type::Func(sig)) => {
                        let code = self.b.ins().load(ptr, MEM, fat, 0);
                        let env = self.b.ins().load(ptr, MEM, fat, 8);
                        (sig.ret, sig.params.as_slice(), env, code)
                    }
                    _ => unreachable!("an indirect callee"),
                };
                let mut sig = self.module.make_signature();
                let mut vals = Vec::new();
                if in_memory(ret) {
                    sig.params.push(AbiParam::new(ptr));
                    vals.push(self.dest_addr(dest));
                }
                sig.params.push(AbiParam::new(ptr));
                vals.push(data);
                for p in params {
                    sig.params.push(AbiParam::new(cl_type(*p).unwrap_or(ptr)));
                }
                sig.returns.extend(cl_type(ret).map(AbiParam::new));
                vals.extend(self.call_args(&args[1..]));
                let sigref = self.b.import_signature(sig);
                let call = self.b.ins().call_indirect(sigref, code, &vals);
                if let (Some(d), true) = (dest, cl_type(ret).is_some()) {
                    let r = self.b.inst_results(call)[0];
                    let (loc, _) = self.locate(d);
                    self.store(loc, r);
                }
            }
            Callee::Func(id) if self.hir.foreign.contains_key(&id) => self.foreign_call(id, args, dest),
            Callee::Func(id) => {
                let ret = self.hir.funcs[id].ret;
                let mut vals = Vec::new();
                if in_memory(ret) {
                    vals.push(self.dest_addr(dest));
                }
                vals.extend(self.call_args(args));
                let callee = self.module.declare_func_in_func(self.ids[id], self.b.func);
                let call = self.b.ins().call(callee, &vals);
                if let (Some(d), true) = (dest, cl_type(ret).is_some()) {
                    let r = self.b.inst_results(call)[0];
                    let (loc, _) = self.locate(d);
                    self.store(loc, r);
                }
            }
            Callee::Str(f) => {
                let mut params = Vec::new();
                let mut vals = Vec::new();
                if in_memory(f.ret()) {
                    params.push(ptr);
                    vals.push(self.dest_addr(dest));
                }
                for (a, &p) in args.iter().zip(f.params()) {
                    params.push(cl_type(p).unwrap_or(ptr));
                    // A `string` parameter is a `&string` operand: already a pointer.
                    vals.push(self.operand(&a.op));
                }
                let returns: Vec<ClType> = cl_type(f.ret()).into_iter().collect();
                let r = self.rt(f.name(), &params, &returns, &vals);
                if let (Some(d), Some(&v)) = (dest, r.first()) {
                    let (loc, _) = self.locate(d);
                    self.store(loc, v);
                }
            }
            Callee::Out { newline } => {
                let p = self.operand(&args[0].op);
                let sptr = self.b.ins().load(ptr, MEM, p, 0);
                let len = self.b.ins().load(i64t, MEM, p, STR_LEN);
                let nl = self.b.ins().iconst(i32t, newline as i64);
                self.rt("drachm_out_str", &[ptr, i64t, i32t], &[], &[sptr, len, nl]);
            }
            Callee::InLine => {
                let out = self.dest_addr(dest);
                self.rt("drachm_in_line", &[ptr], &[], &[out]);
            }
            Callee::System(op) => {
                if op == hir::SystemOp::Platform {
                    let out = self.dest_addr(dest);
                    self.rt(op.symbol(), &[ptr], &[], &[out]);
                } else {
                    let params: Vec<_> = args.iter().map(|arg| self.operand(&arg.op)).collect();
                    let result = self.rt(op.symbol(), &vec![ptr; params.len()], &[types::I8], &params);
                    if let Some(dest) = dest {
                        let (loc, _) = self.locate(dest);
                        self.store(loc, result[0]);
                    }
                }
            }
            Callee::Sleep => {
                let seconds = self.operand(&args[0].op);
                self.rt("drachm_sleep", &[types::F64], &[], &[seconds]);
            }
            Callee::InEnded => {
                let r = self.rt("drachm_in_ended", &[], &[types::I8], &[]);
                if let Some(d) = dest {
                    let (loc, _) = self.locate(d);
                    self.store(loc, r[0]);
                }
            }
            Callee::Parse(ty) => {
                let text = self.operand(&args[0].op);
                let v = match ty {
                    Type::Float(bits) => {
                        let bits = self.b.ins().iconst(i32t, bits as i64);
                        let r = self.rt("drachm_parse_float", &[ptr, i32t], &[types::F64], &[text, bits]);
                        if ty == Type::Float(32) { self.b.ins().fdemote(types::F32, r[0]) } else { r[0] }
                    }
                    Type::Int { bits, signed } => {
                        let signed = self.b.ins().iconst(types::I8, signed as i64);
                        let bits = self.b.ins().iconst(i32t, bits as i64);
                        let r = self.rt("drachm_parse_int", &[ptr, types::I8, i32t], &[i64t, i64t], &[text, signed, bits]);
                        self.join_halves(&r, cl_type(ty).expect("an integer has a machine type"))
                    }
                    _ => unreachable!("checked by typeck"),
                };
                if let Some(d) = dest {
                    let (loc, _) = self.locate(d);
                    self.store(loc, v);
                }
            }
            Callee::ParseStatus(ty) => {
                let text = self.operand(&args[0].op);
                let (kind, signed, bits) = match ty {
                    Type::Float(32) => (1, 0, 32),
                    Type::Float(_) => (2, 0, 64),
                    Type::Int { bits, signed } => (0, signed as i64, bits as i64),
                    _ => unreachable!("checked by typeck"),
                };
                let (kind, signed, bits) = (self.b.ins().iconst(types::I8, kind), self.b.ins().iconst(types::I8, signed), self.b.ins().iconst(i32t, bits));
                let r = self.rt("drachm_parse_status", &[ptr, types::I8, types::I8, i32t], &[types::I8], &[text, kind, signed, bits]);
                if let Some(d) = dest {
                    let (loc, _) = self.locate(d);
                    self.store(loc, r[0]);
                }
            }
            Callee::CauseWrap => {
                let (msg, slot) = (self.operand(&args[0].op), self.operand(&args[1].op));
                let r = self.rt("drachm_cause_wrap", &[ptr, ptr], &[i64t], &[msg, slot]);
                if let Some(d) = dest {
                    let (loc, _) = self.locate(d);
                    self.store(loc, r[0]);
                }
            }
            Callee::ReportExit => {
                let (msg, slot, code) = (self.operand(&args[0].op), self.operand(&args[1].op), self.operand(&args[2].op));
                self.rt("drachm_report_exit", &[ptr, ptr, types::I8], &[], &[msg, slot, code]);
            }
            Callee::Report => {
                let (msg, slot) = (self.operand(&args[0].op), self.operand(&args[1].op));
                self.rt("drachm_report", &[ptr, ptr], &[], &[msg, slot]);
            }
            Callee::Text => {
                let value_ty = self.op_ty(&args[0].op);
                let v = self.operand(&args[0].op);
                let out = self.dest_addr(dest);
                match value_ty {
                    Type::Bool => {
                        self.rt("drachm_text_bool", &[ptr, types::I8], &[], &[out, v]);
                    }
                    Type::Float(32) => {
                        self.rt("drachm_text_f32", &[ptr, types::F32], &[], &[out, v]);
                    }
                    Type::Float(_) => {
                        self.rt("drachm_text_f64", &[ptr, types::F64], &[], &[out, v]);
                    }
                    Type::Int { bits: 128, signed } => {
                        let [lo, hi] = self.halves(v, value_ty);
                        let name = if signed { "drachm_text_s128" } else { "drachm_text_u128" };
                        self.rt(name, &[ptr, i64t, i64t], &[], &[out, lo, hi]);
                    }
                    Type::Int { signed, .. } => {
                        let wide = self.widen64(v, value_ty);
                        let name = if signed { "drachm_text_s64" } else { "drachm_text_u64" };
                        self.rt(name, &[ptr, i64t], &[], &[out, wide]);
                    }
                    _ => unreachable!("checked by typeck"),
                }
            }
            Callee::Radix { base, upper } => {
                let value_ty = self.op_ty(&args[0].op);
                let v = self.operand(&args[0].op);
                let out = self.dest_addr(dest);
                let [lo, hi] = self.halves(v, value_ty);
                let signed = self.b.ins().iconst(types::I8, is_signed(value_ty) as i64);
                let base = self.b.ins().iconst(i32t, base as i64);
                let upper = self.b.ins().iconst(types::I8, upper as i64);
                self.rt("drachm_str_radix", &[ptr, i64t, i64t, types::I8, i32t, types::I8], &[], &[out, lo, hi, signed, base, upper]);
            }
            Callee::ListOp(op, elem) => self.list_op(op, elem, args, dest),
            Callee::MapOp(f, mt) => self.map_call(f, mt, args, dest),
            Callee::ListHas(elem) => self.list_has(elem, args, dest),
            Callee::ListPush(elem) => {
                let list = self.operand(&args[0].op);
                let ety = self.op_ty(&args[1].op);
                let (item, scalar) = if in_memory(ety) {
                    (self.operand_addr(&args[1].op), None)
                } else {
                    let v = self.operand(&args[1].op);
                    let tmp = self.mem(ety);
                    self.b.ins().store(MEM, v, tmp, 0);
                    (tmp, Some(v))
                };
                let (size, align) = self.hir.size_align(elem);
                // Room left: store in place; otherwise the runtime grows the list.
                let (fast, slow, join) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                let len = self.b.ins().load(i64t, MEM, list, STR_LEN);
                let cap = self.b.ins().load(i64t, MEM, list, STR_CAP);
                let room = self.b.ins().icmp(IntCC::UnsignedLessThan, len, cap);
                self.b.ins().brif(room, fast, &[], slow, &[]);
                self.b.switch_to_block(fast);
                let data = self.b.ins().load(ptr, MEM, list, 0);
                let at = if size == 1 { len } else { self.b.ins().imul_imm_u(len, size as i64) };
                let dst = self.b.ins().iadd(data, at);
                match scalar {
                    Some(v) => {
                        self.b.ins().store(MEM, v, dst, 0);
                    }
                    None => self.copy_bits(ety, dst, item),
                }
                let one = self.b.ins().iadd_imm(len, 1);
                self.b.ins().store(MEM, one, list, STR_LEN);
                self.b.ins().jump(join, &[]);
                self.b.switch_to_block(slow);
                let (size, align) = (self.b.ins().iconst(i64t, size as i64), self.b.ins().iconst(i64t, align as i64));
                self.rt("drachm_list_push", &[ptr, ptr, i64t, i64t], &[], &[list, item, size, align]);
                self.b.ins().jump(join, &[]);
                self.b.switch_to_block(join);
            }
            Callee::IndexPanic { unsigned } => {
                let (i, len) = (self.operand(&args[0].op), self.operand(&args[1].op));
                if let Some(cur) = self.b.current_block() {
                    self.b.set_cold_block(cur);
                }
                self.rt(if unsigned { "drachm_index_panic_u" } else { "drachm_index_panic" }, &[i64t, i64t], &[], &[i, len]);
            }
            Callee::Stop(crate::hir::StopKind::ZeroStep) => {
                if let Some(cur) = self.b.current_block() {
                    self.b.set_cold_block(cur);
                }
                self.rt("drachm_range_step_zero", &[], &[], &[]);
            }
            Callee::Stop(crate::hir::StopKind::Assert(_)) => {
                if let Some(cur) = self.b.current_block() {
                    self.b.set_cold_block(cur);
                }
                let at = self.operand_addr(&args[0].op);
                let text = self.b.ins().load(ptr, MEM, at, 0);
                let len = self.b.ins().load(i64t, MEM, at, STR_LEN);
                self.rt("drachm_assert_failed", &[ptr, i64t], &[], &[text, len]);
            }
            Callee::Join => {
                // The parts' values side by side, as the runtime's array.
                let size = STR_SIZE * args.len() as u32;
                let array = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, size.max(1), 3));
                for (i, a) in args.iter().enumerate() {
                    let src = self.operand(&a.op);
                    let at = self.b.ins().stack_addr(ptr, array, (STR_SIZE as usize * i) as i32);
                    self.copy_bits(Type::Str, at, src);
                }
                let base = self.b.ins().stack_addr(ptr, array, 0);
                let n = self.b.ins().iconst(i64t, args.len() as i64);
                let out = self.dest_addr(dest);
                self.rt("drachm_str_join", &[ptr, ptr, i64t], &[], &[out, base, n]);
            }
        }
    }

    // ---- arithmetic ---------------------------------------------------------

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
    /// The address of the drop function of one item of type `elem` for the
    /// list runtime, or null when the item is plain data.
    fn item_drop_fn(&mut self, elem: Type) -> Value {
        let ptr = self.ptr;
        if !needs_drop(self.hir, elem) {
            return self.b.ins().iconst(ptr, 0);
        }
        let id = if elem == Type::Str { import_rt(self.module, self.runtime, "drachm_str_drop", &[ptr], &[]) } else { self.drop_ids[&elem] };
        let f = self.module.declare_func_in_func(id, self.b.func);
        self.b.ins().func_addr(ptr, f)
    }

    /// A list method with a runtime call (see `hir::ListOp`). `args[0]` is the
    /// list's address; the rest are as `build.rs::list_call` made them.
    fn list_op(&mut self, op: hir::ListOp, elem: Type, args: &[Arg], dest: Option<&Place>) {
        use hir::ListOp;
        let (ptr, i64t) = (self.ptr, types::I64);
        let list = self.operand(&args[0].op);
        let (size, align) = self.hir.size_align(elem);
        let size_v = self.b.ins().iconst(i64t, size as i64);
        match op {
            ListOp::Pop | ListOp::View => unreachable!("`pop` is a test and a `Remove`, a view is a borrow, not a call"),
            ListOp::Get => {
                // The item is copied out only when the runtime found one.
                let idx = self.operand(&args[1].op);
                let (tmp, out) = self.item_out(elem, dest);
                let p = self.rt("drachm_list_get", &[ptr, i64t, i64t], &[ptr], &[list, idx, size_v])[0];
                let found = self.b.ins().icmp_imm(IntCC::NotEqual, p, 0);
                let (hit, join) = (self.b.create_block(), self.b.create_block());
                self.b.ins().brif(found, hit, &[], join, &[]);
                self.b.switch_to_block(hit);
                self.copy_item(elem, out, p);
                self.b.ins().jump(join, &[]);
                self.b.switch_to_block(join);
                self.item_done(elem, dest, tmp);
            }
            ListOp::Remove | ListOp::SwapRemove => {
                let idx = self.operand(&args[1].op);
                let (tmp, out) = self.item_out(elem, dest);
                let name = if op == ListOp::Remove { "drachm_list_remove" } else { "drachm_list_swap_remove" };
                self.rt(name, &[ptr, i64t, ptr, i64t], &[], &[list, idx, out, size_v]);
                self.item_done(elem, dest, tmp);
            }
            ListOp::TakeLeaving => {
                let idx = self.operand(&args[1].op);
                let ety = self.op_ty(&args[2].op);
                let item = if in_memory(ety) {
                    self.operand_addr(&args[2].op)
                } else {
                    let v = self.operand(&args[2].op);
                    let tmp = self.mem(ety);
                    self.b.ins().store(MEM, v, tmp, 0);
                    tmp
                };
                let (tmp, out) = self.item_out(elem, dest);
                self.rt("drachm_list_replace", &[ptr, i64t, ptr, ptr, i64t], &[], &[list, idx, item, out, size_v]);
                self.item_done(elem, dest, tmp);
            }
            ListOp::Insert => {
                let idx = self.operand(&args[1].op);
                let ety = self.op_ty(&args[2].op);
                let item = if in_memory(ety) {
                    self.operand_addr(&args[2].op)
                } else {
                    let v = self.operand(&args[2].op);
                    let tmp = self.mem(ety);
                    self.b.ins().store(MEM, v, tmp, 0);
                    tmp
                };
                let align_v = self.b.ins().iconst(i64t, align as i64);
                self.rt("drachm_list_insert", &[ptr, i64t, ptr, i64t, i64t], &[], &[list, idx, item, size_v, align_v]);
            }
            ListOp::Clear => {
                let f = self.item_drop_fn(elem);
                self.rt("drachm_list_clear", &[ptr, i64t, ptr], &[], &[list, size_v, f]);
            }
            ListOp::Join => {
                let sep = self.operand(&args[1].op);
                let out = self.dest_addr(dest);
                self.rt("drachm_list_join", &[ptr, ptr, ptr], &[], &[out, list, sep]);
            }
        }
    }

    /// Where a runtime call writes one item: the destination itself when it
    /// lives in memory, else a fresh slot (its scalar is loaded to the
    /// destination by `item_done`). Returns (the slot if any, the address).
    fn item_out(&mut self, elem: Type, dest: Option<&Place>) -> (Option<Value>, Value) {
        if in_memory(elem) {
            (None, self.dest_addr(dest))
        } else {
            let tmp = self.mem(elem);
            (Some(tmp), tmp)
        }
    }

    /// Finishes `item_out`: a scalar item is loaded from its slot into the destination.
    fn item_done(&mut self, elem: Type, dest: Option<&Place>, tmp: Option<Value>) {
        if let Some(slot) = tmp {
            let cl = cl_type(elem).expect("a scalar item has a machine type");
            let v = self.b.ins().load(cl, MEM, slot, 0);
            self.set_dest(dest, v);
        }
    }

    /// `(&list, index)` as a `ListHas`: whether the runtime finds that item.
    fn list_has(&mut self, elem: Type, args: &[Arg], dest: Option<&Place>) {
        let (ptr, i64t) = (self.ptr, types::I64);
        let (list, idx) = (self.operand(&args[0].op), self.operand(&args[1].op));
        let size = self.b.ins().iconst(i64t, self.hir.size_align(elem).0 as i64);
        let p = self.rt("drachm_list_get", &[ptr, i64t, i64t], &[ptr], &[list, idx, size])[0];
        let found = self.b.ins().icmp_imm(IntCC::NotEqual, p, 0);
        self.set_dest(dest, found);
    }

    /// Stores a call's scalar result in its destination, if it has one.
    fn set_dest(&mut self, dest: Option<&Place>, v: Value) {
        if let Some(d) = dest {
            let (loc, _) = self.locate(d);
            self.store(loc, v);
        }
    }

    fn rt(&mut self, name: &'static str, params: &[ClType], returns: &[ClType], args: &[Value]) -> Vec<Value> {
        // A `drachm_pair` result. SysV returns it in two registers, as declared;
        // Windows x64 returns a 16-byte struct through a hidden first argument.
        if returns.len() == 2 && self.module.isa().default_call_conv() == CallConv::WindowsFastcall {
            let slot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 3));
            let at = self.b.ins().stack_addr(self.ptr, slot, 0);
            let id = import_rt(self.module, self.runtime, name, &[&[self.ptr], params].concat(), &[]);
            let callee = self.module.declare_func_in_func(id, self.b.func);
            self.b.ins().call(callee, &[&[at], args].concat());
            return (0..2).map(|i| self.b.ins().load(returns[i], MEM, at, 8 * i as i32)).collect();
        }
        let id = import_rt(self.module, self.runtime, name, params, returns);
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

    /// An operation under `overflow = saturate | panic`: the runtime does it
    /// on 128-bit values and the result keeps the low bits.
    fn checked(&mut self, op: OvfOp, mode: crate::config::Overflow, l: &Operand, r: &Operand) -> Value {
        let m = if mode == crate::config::Overflow::Saturate { 1 } else { 2 };
        let aty = self.op_ty(l);
        let av = self.operand(l);
        let (i32t, i64t) = (types::I32, types::I64);
        let int_bits = |t: Type| match t.bare() {
            Type::Int { bits, .. } => bits as i64,
            _ => 0,
        };
        let int_signed = |t: Type| matches!(t.bare(), Type::Int { signed: true, .. }) as i64;
        let c32 = |this: &mut Self, v: i64| this.b.ins().iconst(i32t, v);
        let mode_v = c32(self, m);
        match op {
            OvfOp::Convert(to) if aty.is_float() => {
                let d = if matches!(aty.bare(), Type::Float(32)) { self.b.ins().fpromote(types::F64, av) } else { av };
                let (tb, ts) = (c32(self, int_bits(to)), c32(self, int_signed(to)));
                let res = self.rt("drachm_ovf_f2i", &[i32t, i32t, i32t, types::F64], &[i64t, i64t], &[mode_v, tb, ts, d]);
                self.join_halves(&res, cl_type(to).unwrap())
            }
            OvfOp::Convert(to) => {
                let (tb, ts, fs) = (c32(self, int_bits(to)), c32(self, int_signed(to)), c32(self, int_signed(aty)));
                let [lo, hi] = self.halves(av, aty);
                let res = self.rt("drachm_ovf_conv", &[i32t, i32t, i32t, i32t, i64t, i64t], &[i64t, i64t], &[mode_v, tb, ts, fs, lo, hi]);
                self.join_halves(&res, cl_type(to).unwrap())
            }
            OvfOp::Shl | OvfOp::Shr => {
                let bty = self.op_ty(r);
                let bv = self.operand(r);
                let (left, tb, ts) = (c32(self, (op == OvfOp::Shl) as i64), c32(self, int_bits(aty)), c32(self, int_signed(aty)));
                let asg = c32(self, int_signed(bty));
                let [vlo, vhi] = self.halves(av, aty);
                let [nlo, nhi] = self.halves(bv, bty);
                let res = self.rt(
                    "drachm_ovf_shift",
                    &[i32t, i32t, i32t, i32t, i64t, i64t, i64t, i64t, i32t],
                    &[i64t, i64t],
                    &[left, mode_v, tb, ts, vlo, vhi, nlo, nhi, asg],
                );
                self.join_halves(&res, cl_type(aty).unwrap())
            }
            OvfOp::Pow => {
                let xty = self.op_ty(r);
                let xv = self.operand(r);
                let (tb, ts) = (c32(self, int_bits(aty)), c32(self, int_signed(aty)));
                let negative = if int_signed(xty) == 1 {
                    let zero = self.zero(self.b.func.dfg.value_type(xv));
                    let lt = self.b.ins().icmp(IntCC::SignedLessThan, xv, zero);
                    self.b.ins().uextend(i32t, lt)
                } else {
                    c32(self, 0)
                };
                let [blo, bhi] = self.halves(av, aty);
                let [xlo, xhi] = self.halves(xv, xty);
                let res = self.rt(
                    "drachm_ovf_pow",
                    &[i32t, i32t, i32t, i64t, i64t, i64t, i64t, i32t],
                    &[i64t, i64t],
                    &[mode_v, tb, ts, blo, bhi, xlo, xhi, negative],
                );
                self.join_halves(&res, cl_type(aty).unwrap())
            }
            _ => {
                let code = match op {
                    OvfOp::Add => 0,
                    OvfOp::Sub => 1,
                    OvfOp::Mul => 2,
                    OvfOp::Div => 3,
                    _ => 4,
                };
                let (opv, tb, ts) = (c32(self, code), c32(self, int_bits(aty)), c32(self, int_signed(aty)));
                let [alo, ahi] = self.halves(av, aty);
                let [blo, bhi] = if op == OvfOp::Neg {
                    let zero = self.b.ins().iconst(i64t, 0);
                    [zero, zero]
                } else {
                    let bv = self.operand(r);
                    self.halves(bv, aty)
                };
                let res = self.rt(
                    "drachm_ovf_arith",
                    &[i32t, i32t, i32t, i32t, i64t, i64t, i64t, i64t],
                    &[i64t, i64t],
                    &[opv, mode_v, tb, ts, alo, ahi, blo, bhi],
                );
                self.join_halves(&res, cl_type(aty).unwrap())
            }
        }
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

        self.b.switch_to_block(wrap_b);
        let wrapped = if div { self.b.ins().ineg(a) } else { self.int_const(cl, 0) };
        self.b.ins().jump(merge, &[wrapped.into()]);

        self.b.switch_to_block(normal_b);
        let r = if div { self.b.ins().sdiv(a, b) } else { self.b.ins().srem(a, b) };
        self.b.ins().jump(merge, &[r.into()]);

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
                if a == b { v } else if a < b { self.b.ins().fpromote(to_cl, v) } else { self.b.ins().fdemote(to_cl, v) }
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
}

// ---- maps (S6D) ----

/// What the hash and equality of a composite key (a struct, an enum, an
/// array) need to emit code: a walk over its `KeyPlan`.
struct KeyEmit<'a, 'b> {
    module: &'a mut ObjectModule,
    runtime: &'a mut HashMap<&'static str, FuncId>,
    env: GlueEnv<'b>,
    ptr: ClType,
}

fn int_of(size: u32) -> ClType {
    match size {
        1 => types::I8,
        2 => types::I16,
        4 => types::I32,
        8 => types::I64,
        _ => types::I128,
    }
}

impl KeyEmit<'_, '_> {
    fn call_rt(&mut self, b: &mut FunctionBuilder, name: &'static str, params: &[ClType], returns: &[ClType], args: &[Value]) -> Option<Value> {
        let id = import_rt(self.module, self.runtime, name, params, returns);
        let callee = self.module.declare_func_in_func(id, b.func);
        let call = b.ins().call(callee, args);
        b.inst_results(call).first().copied()
    }

    fn combine(&mut self, b: &mut FunctionBuilder, h: Variable, x: Value) {
        let cur = b.use_var(h);
        let next = self.call_rt(b, "drachm_hash_combine", &[types::I64, types::I64], &[types::I64], &[cur, x]).expect("a hash");
        b.def_var(h, next);
    }

    fn at(&self, b: &mut FunctionBuilder, base: Value, off: u32) -> Value {
        if off == 0 { base } else { b.ins().iadd_imm_u(base, off as i64) }
    }

    fn tag(&self, b: &mut FunctionBuilder, base: Value, off: u32, tag_size: u32) -> Value {
        b.ins().load(int_of(tag_size), MEM, base, off as i32)
    }

    /// Runs `key()` on the value at `at`, leaving the result in a fresh slot.
    fn key_result(&mut self, b: &mut FunctionBuilder, func: usize, ret: Type, at: Value) -> Value {
        let (size, align) = self.env.hir.size_align(ret);
        let slot = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, size.max(1), align.trailing_zeros() as u8));
        let out = b.ins().stack_addr(self.ptr, slot, 0);
        let callee = self.module.declare_func_in_func(self.env.ids[func], b.func);
        if in_memory(ret) {
            b.ins().call(callee, &[out, at]);
        } else {
            let call = b.ins().call(callee, &[at]);
            let v = b.inst_results(call)[0];
            b.ins().store(MEM, v, out, 0);
        }
        out
    }

    fn drop_result(&mut self, b: &mut FunctionBuilder, ret: Type, slot: Value) {
        glue_drop_field(self.module, self.runtime, &self.env, b, slot, 0, ret);
    }

    fn hash(&mut self, b: &mut FunctionBuilder, plan: &crate::codegen_common::KeyPlan, base: Value, h: Variable) {
        use crate::codegen_common::KeyPlan;
        match plan {
            KeyPlan::Bytes { off, size } => {
                let x = b.ins().load(int_of(*size), MEM, base, *off as i32);
                if *size == 16 {
                    let (low, high) = b.ins().isplit(x);
                    self.combine(b, h, low);
                    self.combine(b, h, high);
                } else {
                    let x = if *size == 8 { x } else { b.ins().uextend(types::I64, x) };
                    self.combine(b, h, x);
                }
            }
            KeyPlan::Str { off } => {
                let at = self.at(b, base, *off);
                let x = self.call_rt(b, "drachm_str_hash", &[self.ptr], &[types::I64], &[at]).expect("a hash");
                self.combine(b, h, x);
            }
            KeyPlan::Fields(parts) => {
                for p in parts {
                    self.hash(b, p, base, h);
                }
            }
            KeyPlan::Array { off, size, len, item } => {
                let start = self.at(b, base, *off);
                each_item(b, *len, *size, &mut |b, o| {
                    let at = b.ins().iadd(start, o);
                    self.hash(b, item, at, h);
                });
            }
            KeyPlan::Enum { off, tag_size, variants } => {
                let tag = self.tag(b, base, *off, *tag_size);
                let wide = if *tag_size == 8 { tag } else { b.ins().uextend(types::I64, tag) };
                self.combine(b, h, wide);
                for (vi, v) in variants.iter().enumerate() {
                    if matches!(v, KeyPlan::Fields(f) if f.is_empty()) {
                        continue;
                    }
                    let (here, next) = (b.create_block(), b.create_block());
                    let is = b.ins().icmp_imm(IntCC::Equal, tag, vi as i64);
                    b.ins().brif(is, here, &[], next, &[]);
                    b.switch_to_block(here);
                    self.hash(b, v, base, h);
                    b.ins().jump(next, &[]);
                    b.switch_to_block(next);
                }
            }
            KeyPlan::Method { off, func, ret, plan } => {
                let at = self.at(b, base, *off);
                let slot = self.key_result(b, *func, *ret, at);
                self.hash(b, plan, slot, h);
                self.drop_result(b, *ret, slot);
            }
        }
    }

    /// Jumps to `ne` when the values at `a` and `b` differ; falls through when they are equal.
    fn eq(&mut self, bld: &mut FunctionBuilder, plan: &crate::codegen_common::KeyPlan, a: Value, b: Value, ne: ClBlock) {
        use crate::codegen_common::KeyPlan;
        match plan {
            KeyPlan::Bytes { off, size } => {
                let cl = int_of(*size);
                let x = bld.ins().load(cl, MEM, a, *off as i32);
                let y = bld.ins().load(cl, MEM, b, *off as i32);
                let same = bld.ins().icmp(IntCC::Equal, x, y);
                let next = bld.create_block();
                bld.ins().brif(same, next, &[], ne, &[]);
                bld.switch_to_block(next);
            }
            KeyPlan::Str { off } => {
                let (x, y) = (self.at(bld, a, *off), self.at(bld, b, *off));
                let same = self.call_rt(bld, "drachm_str_eq", &[self.ptr, self.ptr], &[types::I8], &[x, y]).expect("a result");
                let next = bld.create_block();
                bld.ins().brif(same, next, &[], ne, &[]);
                bld.switch_to_block(next);
            }
            KeyPlan::Fields(parts) => {
                for p in parts {
                    self.eq(bld, p, a, b, ne);
                }
            }
            KeyPlan::Array { off, size, len, item } => {
                let (start_a, start_b) = (self.at(bld, a, *off), self.at(bld, b, *off));
                each_item(bld, *len, *size, &mut |bld, o| {
                    let (x, y) = (bld.ins().iadd(start_a, o), bld.ins().iadd(start_b, o));
                    self.eq(bld, item, x, y, ne);
                });
            }
            KeyPlan::Enum { off, tag_size, variants } => {
                let (ta, tb) = (self.tag(bld, a, *off, *tag_size), self.tag(bld, b, *off, *tag_size));
                let same = bld.ins().icmp(IntCC::Equal, ta, tb);
                let next = bld.create_block();
                bld.ins().brif(same, next, &[], ne, &[]);
                bld.switch_to_block(next);
                for (vi, v) in variants.iter().enumerate() {
                    if matches!(v, KeyPlan::Fields(f) if f.is_empty()) {
                        continue;
                    }
                    let (here, after) = (bld.create_block(), bld.create_block());
                    let is = bld.ins().icmp_imm(IntCC::Equal, ta, vi as i64);
                    bld.ins().brif(is, here, &[], after, &[]);
                    bld.switch_to_block(here);
                    self.eq(bld, v, a, b, ne);
                    bld.ins().jump(after, &[]);
                    bld.switch_to_block(after);
                }
            }
            KeyPlan::Method { off, func, ret, plan } => {
                let (x, y) = (self.at(bld, a, *off), self.at(bld, b, *off));
                let (ra, rb) = (self.key_result(bld, *func, *ret, x), self.key_result(bld, *func, *ret, y));
                let (differ, done) = (bld.create_block(), bld.create_block());
                self.eq(bld, plan, ra, rb, differ);
                self.drop_result(bld, *ret, ra);
                self.drop_result(bld, *ret, rb);
                bld.ins().jump(done, &[]);
                bld.switch_to_block(differ);
                self.drop_result(bld, *ret, ra);
                self.drop_result(bld, *ret, rb);
                bld.ins().jump(ne, &[]);
                bld.switch_to_block(done);
            }
        }
    }
}

/// The hash and equality functions of a composite key type, from its `KeyPlan`.
#[allow(clippy::too_many_arguments)]
fn define_composite_key_fns(
    module: &mut ObjectModule,
    ctx: &mut cranelift_codegen::Context,
    fctx: &mut FunctionBuilderContext,
    runtime: &mut HashMap<&'static str, FuncId>,
    hir: &hir::Program,
    ids: &[FuncId],
    drop_ids: &HashMap<Type, FuncId>,
    copy_ids: &HashMap<Type, FuncId>,
    map_ops: &HashMap<Type, DataId>,
    key: Type,
    hash: FuncId,
    eq: FuncId,
) -> Result<(), String> {
    let ptr = module.target_config().pointer_type();
    let plan = crate::codegen_common::key_plan(hir, key, 0);
    for (id, is_eq) in [(hash, false), (eq, true)] {
        ctx.func.signature = module.declarations().get_function_decl(id).signature.clone();
        ctx.func.name = UserFuncName::user(0, id.as_u32());
        let cfg = module.isa().frontend_config();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let params = b.block_params(entry).to_vec();
            let mut emit = KeyEmit { module, runtime, env: GlueEnv { hir, ids, drop_ids, copy_ids, map_ops }, ptr };
            if is_eq {
                let ne = b.create_block();
                emit.eq(&mut b, &plan, params[0], params[1], ne);
                let yes = b.ins().iconst(types::I8, 1);
                b.ins().return_(&[yes]);
                b.switch_to_block(ne);
                let no = b.ins().iconst(types::I8, 0);
                b.ins().return_(&[no]);
            } else {
                let h = b.declare_var(types::I64);
                let zero = b.ins().iconst(types::I64, 0);
                b.def_var(h, zero);
                emit.hash(&mut b, &plan, params[0], h);
                let result = b.use_var(h);
                b.ins().return_(&[result]);
            }
            b.seal_all_blocks();
            b.finalize(cfg);
        }
        module.define_function(id, ctx).map_err(|e| format!("in the key functions of `{key}`: {e:?}"))?;
        module.clear_context(ctx);
    }
    Ok(())
}

/// A read-only operations table per (key, value) pair, with relocations to glue.
fn define_map_ops(
    module: &mut ObjectModule,
    runtime: &mut HashMap<&'static str, FuncId>,
    hir: &hir::Program,
    drop_ids: &HashMap<Type, FuncId>,
    copy_ids: &HashMap<Type, FuncId>,
    key_ids: &HashMap<String, (FuncId, FuncId, Type)>,
) -> Result<HashMap<Type, DataId>, String> {
    let mut tables = HashMap::new();
    let ptr = module.target_config().pointer_type();
    for mt in hir.collections.iter().copied().filter(|t| t.is_map()).map(map_ops_type) {
        if tables.contains_key(&mt) {
            continue;
        }
        let (k, v) = mt.map_parts().expect("a map");
        let (ks, ka) = hir.size_align(k);
        let (vs, va) = hir.size_align(v);
        let mut bytes = vec![0; 32 + 6 * ptr.bytes() as usize];
        for (field, n) in bytes.chunks_exact_mut(8).zip([ks, ka, vs, va]) {
            field.copy_from_slice(&(n as u64).to_ne_bytes());
        }
        let mut desc = DataDescription::new();
        desc.set_align(8);
        desc.define(bytes.into_boxed_slice());
        let (hash, eq, _) = key_ids[&key_tag(k)];
        let functions = [
            Some(hash),
            Some(eq),
            glue_fn_id(module, runtime, hir, drop_ids, k, true),
            glue_fn_id(module, runtime, hir, drop_ids, v, true),
            glue_fn_id(module, runtime, hir, copy_ids, k, false),
            glue_fn_id(module, runtime, hir, copy_ids, v, false),
        ];
        for (i, id) in functions.into_iter().enumerate() {
            if let Some(id) = id {
                let reference = module.declare_func_in_data(id, &mut desc);
                desc.write_function_addr(32 + i as u32 * ptr.bytes(), reference);
            }
        }
        let name = format!("drachm_ops_{}", glue_name(mt));
        let data = module.declare_data(&name, Linkage::Local, false, false).map_err(err)?;
        module.define_data(data, &desc).map_err(err)?;
        tables.insert(mt, data);
    }
    Ok(tables)
}

fn map_ops_addr(module: &mut ObjectModule, b: &mut FunctionBuilder, tables: &HashMap<Type, DataId>, mt: Type) -> Value {
    let global = module.declare_data_in_func(tables[&map_ops_type(mt)], b.func);
    b.ins().symbol_value(module.target_config().pointer_type(), global)
}

/// The hash (`hash == true`) or equality function of one key tag: keys hash and
/// compare by their bits, a `string` by its text.
fn define_key_fns(module: &mut ObjectModule, ctx: &mut cranelift_codegen::Context, fctx: &mut FunctionBuilderContext, runtime: &mut HashMap<&'static str, FuncId>, tag: &str, hash: FuncId, eq: FuncId) -> Result<(), String> {
    let ptr = module.target_config().pointer_type();
    let wide = match tag {
        "u128" => types::I128,
        "u8" => types::I8,
        "u16" => types::I16,
        "u32" => types::I32,
        _ => types::I64,
    };
    for (id, is_eq) in [(hash, false), (eq, true)] {
        ctx.func.signature = module.declarations().get_function_decl(id).signature.clone();
        ctx.func.name = UserFuncName::user(0, id.as_u32());
        let cfg = module.isa().frontend_config();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let params = b.block_params(entry).to_vec();
            let result = if tag == "str" {
                let (name, params_ty, ret) = if is_eq { ("drachm_str_eq", vec![ptr, ptr], types::I8) } else { ("drachm_str_hash", vec![ptr], types::I64) };
                let rt = import_rt(module, runtime, name, &params_ty, &[ret]);
                let f = module.declare_func_in_func(rt, b.func);
                let call = b.ins().call(f, &params);
                b.inst_results(call)[0]
            } else if is_eq {
                let x = b.ins().load(wide, MEM, params[0], 0);
                let y = b.ins().load(wide, MEM, params[1], 0);
                // `icmp` gives the 0 or 1 byte the signature returns.
                b.ins().icmp(IntCC::Equal, x, y)
            } else {
                let x = b.ins().load(wide, MEM, params[0], 0);
                let (low, high) = if wide == types::I128 {
                    let (low, high) = b.ins().isplit(x);
                    (low, Some(high))
                } else {
                    let low = if wide == types::I64 { x } else { b.ins().uextend(types::I64, x) };
                    (low, None)
                };
                let rt = import_rt(module, runtime, "drachm_hash_u64", &[types::I64], &[types::I64]);
                let f = module.declare_func_in_func(rt, b.func);
                let call = b.ins().call(f, &[low]);
                let hash = b.inst_results(call)[0];
                if let Some(high) = high {
                    let rt = import_rt(module, runtime, "drachm_hash_combine", &[types::I64, types::I64], &[types::I64]);
                    let f = module.declare_func_in_func(rt, b.func);
                    let call = b.ins().call(f, &[hash, high]);
                    b.inst_results(call)[0]
                } else {
                    hash
                }
            };
            b.ins().return_(&[result]);
            b.seal_all_blocks();
            b.finalize(cfg);
        }
        module.define_function(id, ctx).map_err(|e| format!("in the key functions of `{tag}`: {e:?}"))?;
        module.clear_context(ctx);
    }
    Ok(())
}

impl Cg<'_, '_> {
    /// The address of the `drachm_map_ops` of map type `mt`.
    fn map_ops(&mut self, mt: Type) -> Value {
        map_ops_addr(self.module, &mut self.b, self.map_ops, mt)
    }

    /// The address of a value to pass by pointer: in memory already, or a fresh slot.
    fn operand_at(&mut self, ty: Type, op: &Operand) -> Value {
        if in_memory(ty) {
            self.operand_addr(op)
        } else {
            let v = self.operand(op);
            let tmp = self.mem(ty);
            self.b.ins().store(MEM, v, tmp, 0);
            tmp
        }
    }

    /// Copies one value of `ty` from `src` to `out`: deep when it owns memory.
    fn copy_item(&mut self, ty: Type, out: Value, src: Value) {
        if holds_string(self.hir, ty) {
            if ty == Type::Str {
                self.rt("drachm_str_copy", &[self.ptr, self.ptr], &[], &[out, src]);
            } else {
                let id = self.copy_ids[&ty];
                let f = self.module.declare_func_in_func(id, self.b.func);
                self.b.ins().call(f, &[out, src]);
            }
        } else {
            self.copy_bits(ty, out, src);
        }
    }

    /// A map call (`mir::MapFn`). `args[0]` is the map's address.
    fn map_call(&mut self, f: MapFn, mt: Type, args: &[Arg], dest: Option<&Place>) {
        let (k, v) = mt.map_parts().expect("a map call is on a map");
        let (ptr, i64t) = (self.ptr, types::I64);
        let map = self.operand(&args[0].op);
        match f {
            MapFn::Len => {
                let n = self.rt("drachm_map_len", &[ptr], &[i64t], &[map])[0];
                self.set_dest(dest, n);
            }
            MapFn::Get(result) => {
                let key = self.operand(&args[1].op);
                let ops = self.map_ops(mt);
                let p = self.rt("drachm_map_find", &[ptr, ptr, ptr], &[ptr], &[map, key, ops])[0];
                let (tmp, out) = self.item_out(result, dest);
                let found = self.b.ins().icmp_imm_u(IntCC::NotEqual, p, 0);
                self.b.ins().store(MEM, found, out, 0);
                let present = self.b.create_block();
                let done = self.b.create_block();
                self.b.ins().brif(found, present, &[], done, &[]);
                self.b.switch_to_block(present);
                let offset = self.hir.enum_def(result).variants[1].fields[0].offset;
                let payload = self.b.ins().iadd_imm_u(out, i64::from(offset));
                self.copy_item(v, payload, p);
                self.b.ins().jump(done, &[]);
                self.b.switch_to_block(done);
                self.item_done(result, dest, tmp);
            }
            MapFn::Set => {
                let key = self.operand_at(k, &args[1].op);
                let val = self.operand_at(v, &args[2].op);
                let ops = self.map_ops(mt);
                self.rt("drachm_map_insert", &[ptr, ptr, ptr, ptr], &[types::I8], &[map, key, val, ops]);
            }
            MapFn::Remove => {
                let key = self.operand(&args[1].op);
                let null = self.b.ins().iconst(ptr, 0);
                let ops = self.map_ops(mt);
                self.rt("drachm_map_remove", &[ptr, ptr, ptr, ptr], &[types::I8], &[map, key, null, ops]);
            }
            MapFn::Expect => {
                let key = self.operand(&args[1].op);
                let ops = self.map_ops(mt);
                let p = self.rt("drachm_map_find", &[ptr, ptr, ptr], &[ptr], &[map, key, ops])[0];
                let found = self.b.ins().icmp_imm_u(IntCC::NotEqual, p, 0);
                let (present, missing) = (self.b.create_block(), self.b.create_block());
                self.b.ins().brif(found, present, &[], missing, &[]);
                self.b.switch_to_block(missing);
                self.b.set_cold_block(missing);
                self.rt("drachm_map_key_panic", &[], &[], &[]);
                self.b.ins().trap(TrapCode::unwrap_user(2));
                self.b.switch_to_block(present);
                let (tmp, out) = self.item_out(v, dest);
                self.copy_item(v, out, p);
                self.item_done(v, dest, tmp);
            }
            MapFn::More | MapFn::KeyAt | MapFn::ValAt | MapFn::After => {
                // `drachm_map_next` moves a cursor; it is given a copy, so the cursor value stays.
                let cursor = self.operand(&args[1].op);
                let ops = self.map_ops(mt);
                let slots: Vec<Value> = (0..3).map(|_| self.mem(Type::U64)).collect();
                self.b.ins().store(MEM, cursor, slots[0], 0);
                let found = self.rt("drachm_map_next", &[ptr, ptr, ptr, ptr, ptr], &[types::I8], &[map, slots[0], slots[1], slots[2], ops])[0];
                match f {
                    MapFn::More => self.set_dest(dest, found),
                    MapFn::After => {
                        let after = self.b.ins().load(i64t, MEM, slots[0], 0);
                        self.set_dest(dest, after);
                    }
                    _ => {
                        let (ty, slot) = if f == MapFn::KeyAt { (k, slots[1]) } else { (v, slots[2]) };
                        let at = self.b.ins().load(ptr, MEM, slot, 0);
                        let (tmp, out) = self.item_out(ty, dest);
                        self.copy_item(ty, out, at);
                        self.item_done(ty, dest, tmp);
                    }
                }
            }
        }
    }
}
