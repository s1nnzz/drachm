//! The error code table. `E` codes are errors, `W` codes are warnings; both
//! share one number space, grouped by the first two digits:
//!
//! - `00xx` syntax (lexer and parser)
//! - `01xx` types and conversions
//! - `02xx` names, declarations and bindings
//! - `03xx` ownership and borrowing (step 3)
//! - `09xx` things this compiler version cannot do yet
//!
//! `explain` is the long text shown by `drachm explain <code>`.

#[derive(Debug)]
pub struct Code {
    pub id: &'static str,
    pub title: &'static str,
    pub explain: &'static str,
}

impl Code {
    pub fn is_warning(&self) -> bool {
        self.id.starts_with('W')
    }
}

macro_rules! codes {
    ($($name:ident: $title:literal, $explain:literal;)*) => {
        $(pub static $name: Code = Code { id: stringify!($name), title: $title, explain: $explain };)*
        pub static ALL: &[&Code] = &[$(&$name),*];
    };
}

pub fn lookup(id: &str) -> Option<&'static Code> {
    ALL.iter().copied().find(|c| c.id.eq_ignore_ascii_case(id))
}

codes! {
    E0001: "unexpected character",
        "The source contains a character that is not part of any Drachm token,\n\
         for example `#` or `@` outside a comment or string.";
    E0002: "invalid number",
        "A number literal is malformed or too large: an unknown suffix (`5x`), a\n\
         whole-number suffix on a decimal (`2.5u8`), no digits after `0x`, or a\n\
         constant too large for any number type.\n\n\
         Suffixes are `u8 u16 u32 u64 u128`, `s8 s16 s32 s64 s128`, `f32 f64`,\n\
         and the bare `u` (uint32), `s` (sint32) and `f` (float64).";
    E0003: "unexpected token",
        "The parser needed one thing and found another: a missing `;`, a missing\n\
         `)`, a statement that starts with something that cannot start one, and\n\
         so on. The label shows where, and the message says what was expected.\n\n\
         Conditions are always parenthesised: `if (x) { }`, `while (x) { }`.";
    E0004: "never closed",
        "A `{`, a string or a `/*` comment was opened and never closed. The label\n\
         points at where it was opened.";
    E0005: "not assignable",
        "Only a binding can be assigned to or changed with `++` / `--`.\n\
         `1 + 2 = x;` and `f()++` are not valid.";
    E0006: "parentheses needed",
        "Bitwise operators (`& | ^ << >>`) have no fixed order against arithmetic\n\
         or comparison operators, so mixing them needs parentheses:\n\n\
         \x20   if (a & 1 == 0) { }      // error\n\
         \x20   if ((a & 1) == 0) { }    // ok\n\n\
         Shifts may sit inside `& | ^` (`r << 16 | g << 8 | b`), and a chain of\n\
         one operator needs none (`a | b | c`). Two different ones of `& | ^`,\n\
         or `<<` with `>>`, need parentheses.";
    E0007: "wrong call operator",
        "`::` makes the first call on a value and `->` each call after it:\n\n\
         \x20   x::to<uint8>()->text()";
    E0008: "invalid escape",
        "A string contains a `\\` followed by something that is not an escape.\n\
         The escapes are \\n \\t \\r \\0 \\\\ \\\" \\' \\` \\$ and \\u{1F600}.";

    E0101: "type mismatch",
        "A value of one type was used where another is required.\n\n\
         \x20   bool ok = 5;        // expected bool, found a number\n\
         \x20   if (count) { }      // conditions must be bool\n\n\
         Number types convert to each other automatically, but `bool` is not a\n\
         number and never converts to or from one.";
    W0102: "lossy conversion",
        "A number was converted to a type that cannot hold all of its values,\n\
         or a constant does not fit the type it is going into.\n\n\
         \x20   sint64 big = 1000;\n\
         \x20   uint8 small = big;             // W0102: 1000 becomes 232\n\
         \x20   uint8 sure = big::to<uint8>(); // silent: you asked for it\n\n\
         Out-of-range values wrap around; converting a float to a whole number\n\
         drops the fraction first. In `a + b` the right side converts to the\n\
         left side's type, so this warning also appears on mixed arithmetic.";
    E0103: "wrong number of arguments",
        "A function or method was called with more or fewer arguments (or type\n\
         arguments) than it takes.";
    E0104: "no value here",
        "Something that produces no value was used where a value is needed: the\n\
         result of a `none` function, or a binding declared with type `none`.";
    E0105: "missing return",
        "A function that declares a return type must return a value on every\n\
         path. Either a path reaches the end of the function without `return`,\n\
         or a `return;` gives no value.";
    W0106: "shift amount wraps",
        "A value was shifted by a constant amount that is negative or not\n\
         smaller than its width. The amount wraps around the width: a `uint32`\n\
         shifted by 40 is shifted by 8.";
    E0107: "constant division by zero",
        "A whole-number division or remainder has a constant zero on the right.\n\
         It would always panic, so it is rejected at compile time.";
    E0108: "no such operator",
        "An operator was used on a struct that has no operator method for it.\n\
         The left side leads, so `a + b` calls the `+` method of `a`'s struct:\n\n\
         \x20   struct Vector3 {\n\
         \x20   public:\n\
         \x20       float64 x; float64 y; float64 z;\n\
         \x20       Vector3 +(Vector3 o) { Vector3 r = self.x + o.x, self.y + o.y, self.z + o.z; return r; }\n\
         \x20   }\n\n\
         `==` and `!=` need no method: structs compare field by field unless\n\
         they define `bool ==(T o)`.";
    E0109: "invalid operator method",
        "An operator method has the wrong shape. Operator methods are named by\n\
         one of `+ - * / % **` or `== != < <= > >=`, take exactly one parameter\n\
         (the right-hand side), and are never `static`. Comparisons return\n\
         `bool`, and `==` / `!=` take a value of the struct's own type.";
    E0110: "struct construction",
        "A struct built with the declaration shorthand must get every field\n\
         exactly once, either all in field order or all by name:\n\n\
         \x20   Vector3 a = 1, 2, 3;\n\
         \x20   Vector3 b = z: 3, x: 1, y: 2;\n\
         \x20   Vector3 c = 1, y: 2, z: 3;     // error: mixed\n\
         \x20   Vector3 d = 1, 2;              // error: `z` is missing\n\n\
         Only a struct can be built from a list of values.";

    E0201: "unknown name",
        "A binding, function, method, module or type was used that does not\n\
         exist here. Check the spelling, and that the definition is visible.";
    E0202: "defined twice",
        "Two things that need different names have the same one: two functions\n\
         or two structs in a file, two fields or two methods in a struct, a\n\
         field and a method of one struct, or a struct and a built-in type.";
    E0204: "missing or invalid `main`",
        "A program needs exactly one `main` function that takes no parameters\n\
         and is either `none main()` or `sint32 main()` (which returns the exit\n\
         code).";
    E0205: "changing an immutable binding",
        "Bindings and parameters cannot change unless declared `mut`.\n\n\
         \x20   uint32 a = 1;\n\
         \x20   a = 2;              // error\n\
         \x20   mut uint32 b = 1;\n\
         \x20   b = 2;              // ok\n\n\
         A binding declared without a value (`uint32 later;`) may be assigned\n\
         exactly once, where it certainly has no value yet.\n\n\
         A struct's fields can change when the binding is `mut` or the field\n\
         itself is `mut` (a `mut` field can always change):\n\n\
         \x20   struct Player { public: uint32 level; mut uint32 hp; }\n\
         \x20   Player p = 1, 100;\n\
         \x20   p.hp -= 10;         // ok: `hp` is a `mut` field\n\
         \x20   p.level = 2;        // error: neither `p` nor `level` is `mut`\n\n\
         A method that changes a non-`mut` field of `self` (or calls a method\n\
         that does) changes its value, so it can only be called on a `mut`\n\
         binding, never on a temporary value.";
    E0206: "module not imported",
        "A module was used without importing it. Nothing is imported\n\
         automatically, not even `io`: add `in io;` at the top of the file.";
    E0207: "read before assignment",
        "A binding declared without a value was read on a path where it may\n\
         not have been given one yet.\n\n\
         \x20   uint32 x;\n\
         \x20   if (ready) { x = 1; }\n\
         \x20   io::outln(x::text());   // error: no value if `ready` is false";
    W0208: "method does not use `self`",
        "A method that never uses `self` does not need a value to be called on.\n\
         Mark it `static` and call it on the struct instead:\n\n\
         \x20   static Point origin() { Point p = 0, 0; return p; }\n\
         \x20   Point o = Point::origin();";
    E0209: "no `self` here",
        "`self` is the value a method was called on. It exists only inside a\n\
         struct's methods, and not in `static` ones, which are called on the\n\
         struct (`Point::origin()`) rather than on a value.";
    E0210: "struct contains itself",
        "A struct has a field of its own type, directly or through other\n\
         structs (`A` holds a `B`, which holds an `A`). Its size would never\n\
         end, so this is not allowed.";
    E0211: "field of an unassigned struct",
        "A single field was set on a struct binding that may not have a value\n\
         yet. Give the binding a whole value first:\n\n\
         \x20   Point p;\n\
         \x20   p.x = 1;            // error\n\
         \x20   p = q;              // ok: the whole value";
    E0212: "private member",
        "A field or method in a struct's private section was used outside that\n\
         struct's own methods. Before any `public:` or `private:` label, fields\n\
         are private and methods are public.";
    E0213: "static and non-static calls",
        "A `static` method is called on the struct (`Point::origin()`); every\n\
         other method is called on a value (`p::length()`). This call used the\n\
         wrong one.";

    E0901: "not supported yet",
        "This is part of Drachm's design, but this version of the compiler does\n\
         not implement it yet.";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_well_formed() {
        for (i, c) in ALL.iter().enumerate() {
            assert!(c.id.len() == 5 && (c.id.starts_with('E') || c.id.starts_with('W')), "{}", c.id);
            assert!(c.id[1..].bytes().all(|b| b.is_ascii_digit()), "{}", c.id);
            // E and W share one number space.
            assert!(ALL[..i].iter().all(|o| o.id[1..] != c.id[1..]), "duplicate {}", c.id);
        }
    }
}
