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
/// The list and map interface; it uses drachm_str, so it follows strings.h.
const LISTS_H: &str = include_str!("../runtime/lists.h");

/// The runtime and the constant helpers every program starts with.
pub(crate) fn prelude() -> String {
    let mut out = String::from("#define DRACHM_INLINE 1\n");
    // Without the line ending, which is "\r\n" in a Windows checkout.
    out += &RUNTIME_C.replace("#include \"strings.h\"", STRINGS_H);
    out += "\n";
    out += LISTS_H;
    // A borrow of an interface (`&Shape`): the value's address and its table.
    out += "\ntypedef struct { void *data; const void *const *vt; } drachm_dyn;\ntypedef struct { void (*code)(void); uint8_t *env; } drachm_fn;\n";
    out += "\nstatic inline double drachm_f64_bits(uint64_t b) { double d; memcpy(&d, &b, 8); return d; }\n";
    out += "static inline float drachm_f32_bits(uint32_t b) { float f; memcpy(&f, &b, 4); return f; }\n\n";
    out
}

/// Every struct's and enum's definition (and, for owning ones, its copy and
/// drop functions). With `asserts`, each definition is followed by a
/// `_Static_assert` that the C compiler lays it out as `hir::layout` did.
/// An enum is a tag and a union of one struct per variant that has values.
pub(crate) fn struct_defs(prog: &Program, asserts: bool) -> String {
    let mut out = String::new();
    // Every struct is named up front: a pointer to one may come before its definition.
    for def in &prog.structs {
        let name = struct_name(&def.name);
        let _ = writeln!(out, "typedef struct {name} {name};");
    }
    for item in type_order(prog) {
        match item {
            Item::Struct(s) => {
                let def = &prog.structs[s];
                let name = struct_name(&def.name);
                let _ = writeln!(out, "struct {name} {{");
                for f in &def.fields {
                    let _ = writeln!(out, "    {} f_{};", c_type(f.ty), f.name);
                }
                let _ = writeln!(out, "}};\n");
                if asserts {
                    let mut checks = format!("sizeof({name}) == {}", def.size);
                    for f in &def.fields {
                        let _ = write!(checks, " && __builtin_offsetof({name}, f_{}) == {}", f.name, f.offset);
                    }
                    let _ = writeln!(out, "_Static_assert({checks}, \"layout\");\n");
                }
            }
            Item::Array(t) => {
                let Type::Array { elem, len } = t else { unreachable!() };
                let name = array_name(t);
                let _ = writeln!(out, "typedef struct {{ {} a[{len}]; }} {name};\n", c_type(*elem));
                if asserts {
                    let (size, align) = prog.size_align(t);
                    let _ = writeln!(out, "_Static_assert(sizeof({name}) == {size} && _Alignof({name}) == {align}, \"layout\");\n");
                }
            }
            Item::Enum(e) => {
                let def = &prog.enums[e];
                let name = enum_name(e);
                let _ = writeln!(out, "typedef struct {{\n    {} tag;", c_type(crate::mir::ty::tag_type(def.tag_size)));
                if def.variants.iter().any(|v| !v.fields.is_empty()) {
                    let _ = writeln!(out, "    union {{");
                    for (vi, v) in def.variants.iter().enumerate() {
                        if v.fields.is_empty() {
                            continue;
                        }
                        let _ = write!(out, "        struct {{");
                        for f in &v.fields {
                            let _ = write!(out, " {} f_{};", c_type(f.ty), f.name);
                        }
                        let _ = writeln!(out, " }} v{vi};");
                    }
                    let _ = writeln!(out, "    }} u;");
                }
                let _ = writeln!(out, "}} {name};\n");
                if asserts {
                    let mut checks = format!("sizeof({name}) == {} && _Alignof({name}) == {}", def.size, def.align);
                    for (vi, v) in def.variants.iter().enumerate() {
                        for f in &v.fields {
                            let _ = write!(checks, " && __builtin_offsetof({name}, u.v{vi}.f_{}) == {}", f.name, f.offset);
                        }
                    }
                    let _ = writeln!(out, "_Static_assert({checks}, \"layout\");\n");
                }
            }
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

/// The C type of enum `id`.
pub(crate) fn enum_name(id: usize) -> String {
    format!("drachm_e{id}")
}

/// The C struct that wraps the array type `t` (`uint8[4]` is `drachm_a_uint8_A4`),
/// so that arrays assign, pass and return by value like everything else.
pub(crate) fn array_name(t: Type) -> String {
    format!("drachm_a_{}", crate::types::mangle(&t.to_string()))
}

pub(crate) fn struct_name(name: &str) -> String {
    format!("drachm_s_{}", crate::types::mangle(name))
}

/// A struct or an enum, for ordering definitions.
#[derive(Clone, Copy)]
enum Item {
    Struct(StructId),
    Enum(EnumId),
    Array(Type),
}

/// Every struct and enum, each after the ones it contains (C needs a
/// definition before it is used as a field).
fn type_order(prog: &Program) -> Vec<Item> {
    fn visit_type(prog: &Program, t: Type, done: &mut (Vec<bool>, Vec<bool>), order: &mut Vec<Item>) {
        match t {
            Type::Struct { id, .. } => visit(prog, Item::Struct(id), done, order),
            Type::Enum { id, .. } => visit(prog, Item::Enum(id), done, order),
            // An array holds its items by value, so they are defined first.
            Type::Array { elem, .. } => {
                visit_type(prog, *elem, done, order);
                if !order.iter().any(|i| matches!(i, Item::Array(a) if *a == t)) {
                    order.push(Item::Array(t));
                }
            }
            _ => {}
        }
    }
    fn visit(prog: &Program, item: Item, done: &mut (Vec<bool>, Vec<bool>), order: &mut Vec<Item>) {
        match item {
            Item::Struct(s) => {
                if std::mem::replace(&mut done.0[s], true) {
                    return;
                }
                for f in &prog.structs[s].fields {
                    visit_type(prog, f.ty, done, order);
                }
            }
            Item::Array(_) => return,
            Item::Enum(e) => {
                if std::mem::replace(&mut done.1[e], true) {
                    return;
                }
                for f in prog.enums[e].variants.iter().flat_map(|v| &v.fields) {
                    visit_type(prog, f.ty, done, order);
                }
            }
        }
        order.push(item);
    }
    let mut done = (vec![false; prog.structs.len()], vec![false; prog.enums.len()]);
    let mut order = Vec::new();
    for s in 0..prog.structs.len() {
        visit(prog, Item::Struct(s), &mut done, &mut order);
    }
    for e in 0..prog.enums.len() {
        visit(prog, Item::Enum(e), &mut done, &mut order);
    }
    for &t in &prog.collections {
        visit_type(prog, t, &mut done, &mut order);
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
        Type::List { .. } => "drachm_list",
        Type::Map { .. } => "drachm_map",
        Type::Array { .. } => return array_name(t),
        Type::None => "void",
        Type::Unit { .. } => unreachable!("units are erased before the backends"),
        Type::Val { .. } => unreachable!("a value argument is no type of the program"),
        Type::C(_) | Type::CEnum { .. } => unreachable!("C types are erased before the backends"),
        Type::Struct { name, .. } => return struct_name(name),
        Type::Enum { id, .. } => return enum_name(id),
        Type::Ref { to: Type::Dyn { .. }, .. } => "drachm_dyn",
        Type::Func(_) => "drachm_fn",
        Type::Dyn { .. } => unreachable!("an interface is only the target of a borrow"),
        Type::Ref { to, .. } | Type::Ptr { to } | Type::FixedPtr { to } | Type::Foreign { to } => return format!("{} *", c_type(*to)),
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
