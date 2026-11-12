//! Maps (S6D): `Map<K, V>` and `OrderedMap<K, V>`, hash tables of owned keys
//! and values. The runtime is `runtime/lists.h` (`drachm_map_*`).
//!
//! Step 1 (this file): the type names, the key rule and the layout. A key is
//! a whole number, a float (hashed and compared by its bits, Q14), a `bool` or
//! a `string`. Other key types wait (E0901) until their hashing is built.

use super::Globals;
use crate::ast;
use crate::diag::{Diagnostic, codes};
use crate::types::Type;

/// Is `t` written as `Map<K, V>` or `OrderedMap<K, V>` (and not a program's
/// own type of that name)?
pub(super) fn is_map_name(g: &Globals, t: &ast::TypeName) -> bool {
    matches!(t.name.as_str(), "Map" | "OrderedMap")
        && t.args.len() == 2
        && !g.structs_by_name.contains_key(&t.name)
        && !g.enums_by_name.contains_key(&t.name)
        && !g.mono.type_templates.contains_key(&t.name)
}

impl Globals {
    /// `Map<key, val>`, noted for the backends.
    pub(super) fn make_map(&self, key: Type, val: Type, ordered: bool) -> Type {
        let t = Type::map(key, val, ordered);
        self.note_collection(t);
        t
    }
}

/// Can a value of type `key` be a map key (step 1)? `Err` says why not, as
/// the diagnostic to report.
pub(super) fn check_key(key: Type, at: crate::diag::Span) -> Result<(), Diagnostic> {
    match key.bare() {
        Type::Int { .. } | Type::Float(_) | Type::Bool | Type::Str => Ok(()),
        Type::List { .. } | Type::Array { .. } | Type::Map { .. } | Type::Ref { .. } | Type::Ptr { .. } | Type::FixedPtr { .. } => Err(
            Diagnostic::at(&codes::E0626, at, format!("`{key}` cannot be a map key"))
                .note(format!("`{key}` is not hashed"))
                .help("use a whole number, a float, `bool` or `string` as the key"),
        ),
        _ => Err(Diagnostic::at(&codes::E0901, at, format!("`{key}` as a map key is not supported yet")).note("only numbers, `bool` and `string` are hashed so far")),
    }
}

impl Globals {
    /// `Map<K, V>` or `OrderedMap<K, V>` as written (the arguments resolved
    /// here; the `?`, `&` and `*` marks are applied by the caller).
    pub(super) fn resolve_map(&self, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let ordered = t.name == "OrderedMap";
        let key = self.resolve(&t.args[0], diags)?;
        let val = self.resolve(&t.args[1], diags)?;
        let at = t.args[0].span;
        if let Err(d) = check_key(key, at) {
            diags.push(d);
            return None;
        }
        if val == Type::None {
            diags.push(Diagnostic::at(&codes::E0104, t.args[1].span, "a map cannot hold `none`").note("`none` holds no value"));
            return None;
        }
        if val.is_borrow() || self.holds_borrow(val) || key.is_borrow() {
            diags.push(
                Diagnostic::at(&codes::E0901, t.span, format!("a `{}` of borrows is not supported yet", t.name))
                    .note("this map would hold a borrow")
                    .help("keep the values themselves in the map, and borrow them where you use them"),
            );
            return None;
        }
        Some(self.make_map(key, val, ordered))
    }
}
