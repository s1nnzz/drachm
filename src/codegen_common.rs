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
        // A fat borrow is two words in memory.
        Type::Ref { to: Type::Dyn { .. }, .. } | Type::Func(_) => None,
        Type::Ref { .. } | Type::Ptr { .. } | Type::FixedPtr { .. } | Type::Foreign { .. } => Some(types::I64),
        Type::Dyn { .. } => unreachable!("an interface is only the target of a borrow"),
        Type::None | Type::Str | Type::List { .. } | Type::Map { .. } | Type::Array { .. } | Type::Struct { .. } | Type::Enum { .. } => None,
        Type::Unit { .. } => unreachable!("units are erased before the backends"),
        Type::Val { .. } => unreachable!("a value argument is no type of the program"),
        Type::C(_) => unreachable!("C types are erased before the backends"),
    }
}

/// Strings and structs live in memory and are handled by address.
pub(crate) fn in_memory(t: Type) -> bool {
    t == Type::Str || t.is_list() || t.is_map() || t.is_array() || t.struct_id().is_some() || t.enum_id().is_some() || t.is_fat() || t.is_func()
}

pub(crate) fn signature(module: &ObjectModule, f: &Func) -> Signature {
    let ptr = module.target_config().pointer_type();
    let mut s = module.make_signature();
    if in_memory(f.ret) {
        s.params.push(AbiParam::new(ptr));
    }
    // A method's `self` is passed as a borrow of the receiver, whatever its type
    // (a struct is in memory anyway; a number added to by `uint32 with I { }` is not).
    s.params.extend(f.locals[..f.params].iter().enumerate().map(|(i, l)| AbiParam::new(if i == 0 && f.has_self { ptr } else { cl_type(l.ty).unwrap_or(ptr) })));
    s.returns.extend(cl_type(f.ret).map(AbiParam::new));
    s
}

/// The signature of a C function (step 7.1): the target's C calling
/// convention, with the narrow integers extended the way the C ABI expects
/// from a caller.
pub(crate) fn foreign_signature(module: &ObjectModule, f: &Func) -> Signature {
    let mut s = module.make_signature();
    s.params.extend(f.locals[..f.params].iter().map(|l| {
        let ty = cl_type(l.ty).expect("a C function takes numbers and pointers");
        let p = AbiParam::new(ty);
        match l.ty {
            Type::Int { bits: 8 | 16, signed: true } => p.sext(),
            Type::Int { bits: 8 | 16, signed: false } | Type::Bool => p.uext(),
            _ => p,
        }
    }));
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

/// The key function tag of a map key type (S6D). Scalar keys hash and compare
/// by their bits, so a tag is the key's width; a `string` has its own, and any
/// other key (a struct, an enum, an array) is named by its type.
pub fn key_tag(k: crate::types::Type) -> String {
    use crate::types::Type;
    let t = k.bare();
    match t {
        Type::Str => "str".into(),
        Type::Int { .. } | Type::Float(_) | Type::Bool => match crate::hir::scalar_size(t) {
            Some(1) => "u8".into(),
            Some(2) => "u16".into(),
            Some(4) => "u32".into(),
            Some(8) => "u64".into(),
            _ => "u128".into(),
        },
        _ => format!("k_{}", crate::types::mangle(&t.to_string())),
    }
}

/// Is the key hashed by a plan (see `KeyPlan`) rather than by its width?
pub fn key_is_composite(k: crate::types::Type) -> bool {
    key_tag(k).starts_with("k_")
}

/// How a map key is hashed and compared, built from the key's layout. Offsets
/// are from the base of the value the plan is for.
pub enum KeyPlan {
    /// A number or `bool` of this many bytes, by its bits.
    Bytes { off: u32, size: u32 },
    Str { off: u32 },
    /// Each part in order.
    Fields(Vec<KeyPlan>),
    /// `len` items of `size` bytes from `off`; `item` is relative to an item.
    Array { off: u32, size: u32, len: u64, item: Box<KeyPlan> },
    /// The tag (`tag_size` bytes at `off`), then the plan of the variant it names.
    Enum { off: u32, tag_size: u32, variants: Vec<KeyPlan> },
    /// A struct with `key()`: the key is what `func` returns for the value at
    /// `off`; that result is of type `ret` and keyed by `plan`.
    Method { off: u32, func: usize, ret: crate::types::Type, plan: Box<KeyPlan> },
}

pub fn key_plan(hir: &crate::hir::Program, ty: crate::types::Type, off: u32) -> KeyPlan {
    use crate::types::Type;
    match ty.bare() {
        Type::Str => KeyPlan::Str { off },
        Type::Struct { id, .. } => {
            let def = &hir.structs[id];
            match def.key_fn {
                Some(func) => {
                    let ret = hir.funcs[func].ret;
                    KeyPlan::Method { off, func, ret, plan: Box::new(key_plan(hir, ret, 0)) }
                }
                None => KeyPlan::Fields(def.fields.iter().map(|f| key_plan(hir, f.ty, off + f.offset)).collect()),
            }
        }
        t @ Type::Enum { .. } => {
            let def = hir.enum_def(t);
            let variants = def.variants.iter().map(|v| KeyPlan::Fields(v.fields.iter().map(|f| key_plan(hir, f.ty, off + f.offset)).collect())).collect();
            KeyPlan::Enum { off, tag_size: def.tag_size, variants }
        }
        Type::Array { elem, len } => {
            let (size, _) = hir.size_align(*elem);
            KeyPlan::Array { off, size, len: len, item: Box::new(key_plan(hir, *elem, 0)) }
        }
        t => KeyPlan::Bytes { off, size: crate::hir::scalar_size(t).expect("a hashable key is made of numbers, text, structs, enums and arrays") },
    }
}

/// Ordered and unordered maps with the same key and value share runtime operations.
pub fn map_ops_type(map: crate::types::Type) -> crate::types::Type {
    let (key, value) = map.map_parts().expect("a map");
    crate::types::Type::map(key, value, false)
}
