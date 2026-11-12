//! What the Cranelift backend (`codegen_mir`) shares with the entry glue: the
//! value classes, the call signature, the entry point.
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

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlagsData, Signature, UserFuncName};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::ObjectModule;

use crate::hir::*;
use crate::types::Type;

type ClType = types::Type;

/// Loads and stores of strings and structs: always aligned, never trapping.
pub(crate) const MEM: MemFlagsData = MemFlagsData::trusted();

/// Offsets of a `drachm_str`'s length and capacity.
pub(crate) const STR_LEN: i32 = 8;
pub(crate) const STR_CAP: i32 = 16;

/// The Cranelift type of a number or bool; `None` for everything else.
pub(crate) fn cl_type(t: Type) -> Option<ClType> {
    match t {
        Type::Int { bits, .. } => ClType::int(bits),
        Type::Float(32) => Some(types::F32),
        Type::Float(_) => Some(types::F64),
        Type::Bool => Some(types::I8),
        Type::Ref { .. } | Type::Ptr { .. } | Type::FixedPtr { .. } => Some(types::I64),
        Type::None | Type::Str | Type::List { .. } | Type::Array { .. } | Type::Struct { .. } | Type::Enum { .. } => None,
        Type::Unit { .. } => unreachable!("units are erased before the backends"),
    }
}

/// Strings and structs live in memory and are handled by address.
pub(crate) fn in_memory(t: Type) -> bool {
    t == Type::Str || t.is_list() || t.is_array() || t.struct_id().is_some() || t.enum_id().is_some()
}

pub(crate) fn signature(module: &ObjectModule, f: &Func) -> Signature {
    let ptr = module.target_config().pointer_type();
    let mut s = module.make_signature();
    if in_memory(f.ret) {
        s.params.push(AbiParam::new(ptr));
    }
    s.params.extend(f.locals[..f.params].iter().map(|l| AbiParam::new(cl_type(l.ty).unwrap_or(ptr))));
    s.returns.extend(cl_type(f.ret).map(AbiParam::new));
    s
}

pub(crate) fn is_signed(t: Type) -> bool {
    matches!(t, Type::Int { signed: true, .. })
}

pub(crate) fn err(e: impl std::fmt::Debug) -> String {
    format!("{e:?}")
}

/// `int main(void)` that calls the Drachm `main` and turns its result into an exit code.
pub(crate) fn emit_c_main(
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
