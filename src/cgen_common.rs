//! What the C backend (`cgen_mir`) shares: the prelude with the runtime, the
//! struct definitions, C type names and the helpers every arithmetic
//! operation uses.
//!
//! Strings are `drachm_str` structs and Drachm structs are C structs (fields
//! `f_<name>`). Both are passed to and returned from Drachm functions by
//! value (the callee owns its parameters), except a method's `self`, which
//! is a pointer to the caller's value; strings go to the runtime by pointer.

use std::fmt::Write;

use crate::hir::*;
use crate::types::Type;

const RUNTIME_C: &str = include_str!("../runtime/runtime.c");
const STRINGS_H: &str = include_str!("../runtime/strings.h");

/// The runtime and the constant helpers every program starts with.
pub(crate) fn prelude() -> String {
    let mut out = String::from("#define DRACHM_INLINE 1\n");
    out += &RUNTIME_C.replace("#include \"strings.h\"\n", STRINGS_H);
    out += "\nstatic inline double drachm_f64_bits(uint64_t b) { double d; memcpy(&d, &b, 8); return d; }\n";
    out += "static inline float drachm_f32_bits(uint32_t b) { float f; memcpy(&f, &b, 4); return f; }\n\n";
    out
}

/// Every struct's definition (and, for owning ones, its copy and drop
/// functions). With `asserts`, each definition is followed by a
/// `_Static_assert` that the C compiler lays it out as `hir::layout` did.
pub(crate) fn struct_defs(prog: &Program, asserts: bool) -> String {
    let mut out = String::new();
    for s in struct_order(prog) {
        let def = &prog.structs[s];
        let name = struct_name(&def.name);
        let _ = writeln!(out, "typedef struct {{");
        for f in &def.fields {
            let _ = writeln!(out, "    {} f_{};", c_type(f.ty), f.name);
        }
        let _ = writeln!(out, "}} {name};\n");
        if asserts {
            let mut checks = format!("sizeof({name}) == {}", def.size);
            for f in &def.fields {
                let _ = write!(checks, " && __builtin_offsetof({name}, f_{}) == {}", f.name, f.offset);
            }
            let _ = writeln!(out, "_Static_assert({checks}, \"layout\");\n");
        }
    }
    out
}

/// `int main(void)`, which calls the Drachm `main` and turns its result into an exit code.
pub(crate) fn main_fn(prog: &Program) -> String {
    let main = &prog.funcs[prog.main];
    let call = format!("{}()", main.symbol);
    let body = if main.ret == Type::None { format!("{call}; return 0;") } else { format!("return (int){call};") };
    format!("\nint main(void) {{ {body} }}\n")
}

pub(crate) fn struct_name(name: &str) -> String {
    format!("drachm_s_{name}")
}

/// Every struct, each after the structs it contains (C needs a struct's
/// definition before it is used as a field).
fn struct_order(prog: &Program) -> Vec<StructId> {
    fn visit(prog: &Program, s: StructId, done: &mut [bool], order: &mut Vec<StructId>) {
        if done[s] {
            return;
        }
        done[s] = true;
        for f in &prog.structs[s].fields {
            if let Some(inner) = f.ty.struct_id() {
                visit(prog, inner, done, order);
            }
        }
        order.push(s);
    }
    let mut done = vec![false; prog.structs.len()];
    let mut order = Vec::new();
    for s in 0..prog.structs.len() {
        visit(prog, s, &mut done, &mut order);
    }
    order
}

pub(crate) fn c_type(t: Type) -> String {
    let name = match t {
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
        Type::Str => "drachm_str",
        Type::None => "void",
        Type::Unit { .. } => unreachable!("units are erased before the backends"),
        Type::Struct { name, .. } => return struct_name(name),
        Type::Ref { to, .. } | Type::Ptr { to } => return format!("{} *", c_type(*to)),
    };
    name.to_string()
}

pub(crate) fn bits(t: Type) -> u16 {
    match t {
        Type::Int { bits, .. } => bits,
        _ => unreachable!("not an integer type"),
    }
}

pub(crate) fn signed(t: Type) -> bool {
    matches!(t, Type::Int { signed: true, .. })
}

/// The unsigned type integer arithmetic on `t` is carried out in: at least
/// 32 bits, so that C's promotion to (signed) `int` never gets involved.
pub(crate) fn work_type(t: Type) -> &'static str {
    match bits(t) {
        8 | 16 | 32 => "uint32_t",
        64 => "uint64_t",
        _ => "drachm_u128",
    }
}

/// `v`'s low and high 64-bit halves, sign- or zero-extended from its own type.
pub(crate) fn halves(v: &str, t: Type) -> String {
    let wide = if signed(t) { "drachm_s128" } else { "drachm_u128" };
    format!("(uint64_t)({v}), (uint64_t)((({wide})({v})) >> 64)")
}

/// A literal as a `drachm_str` initialiser: a not-owned view of read-only bytes.
pub(crate) fn c_str_value(s: &str) -> String {
    if s.is_empty() { "{0}".to_string() } else { format!("{{(uint8_t *){}, {}, 0}}", c_string(s), s.len()) }
}

pub(crate) fn c_string(s: &str) -> String {
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
