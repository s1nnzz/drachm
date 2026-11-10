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
         The escapes are \\n \\t \\r \\0 \\\\ \\\" \\' \\` \\$ \\% and \\u{1F600}.";
    E0009: "invalid format placeholder",
        "An `io::outf` format has a `%` placeholder that cannot be read. A bare `%`\n\
         prints the next value as its `::text()`. Options go between the `%` and\n\
         the value, in this order:\n\n\
         \x20   %8   %-8   %08      pad to 8 characters: on the left, on the right,\n\
         \x20                       or with zeros after the sign\n\
         \x20   %.2               two decimal places (floats)\n\
         \x20   %x %X %b %o       hex, capital hex, binary, octal (whole numbers)\n\n\
         For example `%08x` or `%-10.2`. A letter other than x, X, b, o right after\n\
         a placeholder is an error, and so is `%%`: a percent sign is written `\\%`.";
    E0010: "range out of place",
        "An inclusive range `a..b` was used somewhere other than the two places\n\
         ranges can go so far: after `of` in a `for` loop and as a `case`\n\
         pattern.\n\n\
         \x20   for (uint32 i of 0..9) { }     // ok: 0 to 9\n\
         \x20   match (n) { case 1..5 { } }    // ok\n\
         \x20   uint32 r = 0..9;               // error";
    E0011: "no loop to leave",
        "`break` and `continue` work on loops. `break N;` and `continue N;` count\n\
         N loops outward (default 1), so N must be at least 1 and at most the\n\
         number of loops around the statement.\n\n\
         \x20   while (a) {\n\
         \x20       for (uint32 i of 0..9) {\n\
         \x20           continue 2;            // next round of `while (a)`\n\
         \x20           break 3;               // error: only 2 loops\n\
         \x20       }\n\
         \x20   }\n\n\
         To leave an `if`, use `skip`; to leave the function, `return`.";
    E0012: "nothing to skip",
        "`skip` leaves `if` blocks the way `break` leaves loops. A whole\n\
         `if` / `else if` / `else` chain is one level, and so is a `match` (each\n\
         `case` is a branch of it). Bare `skip;` leaves the outermost level,\n\
         `skip N;` leaves N levels counting outward from the innermost, and\n\
         `skip to next;` goes on to the next branch of the innermost level.\n\n\
         Loops are walls: only the levels inside the current loop body count\n\
         (or inside the function body, outside any loop). This error means\n\
         there is no level to leave, N is 0 or larger than the number of\n\
         levels, or `skip to next` is in the last branch, which has no next one.";
    E0013: "invalid `for` header",
        "The step of a `for (init; condition; step)` loop must be an assignment,\n\
         a compound assignment, `++` / `--`, or a call:\n\n\
         \x20   for (mut uint32 i = 0; i < 10; i += 2) { }";

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
    E0108: "format option does not suit the value",
        "An `io::outf` placeholder asks for something its value cannot do:\n\n\
         \x20   io::outf(\"%.2\", count);    // decimal places need a float\n\
         \x20   io::outf(\"%x\", ratio);     // a base needs a whole number\n\
         \x20   io::outf(\"%08\", name);     // zero padding needs a number";
    E0109: "invalid pattern",
        "A `case` pattern must be a constant of the matched type: a number or\n\
         constant expression (`case 3`, `case -1`, `case 2 ** 8`), `true` or\n\
         `false`, an inclusive range of numbers whose start is not above its\n\
         end (`case 13..19`), or `_` on its own (`case _`), which matches\n\
         anything.";
    W0110: "missing case",
        "A `match` has values that no `case` handles, so for those nothing\n\
         runs. Add the missing cases, or `case _ { }` for everything else.\n\n\
         \x20   match (flag) {\n\
         \x20       case true { }\n\
         \x20   }                             // W0110: `false` is not handled\n\n\
         On float values only `case _` handles everything (NaN equals nothing).";
    W0111: "unreachable case",
        "A `case` can never run: it comes after `case _`, or every value it\n\
         matches is already handled by the cases before it. Cases are tested in\n\
         order and the first one that matches runs.";

    E0201: "unknown name",
        "A binding, function, method, module or type was used that does not\n\
         exist here. Check the spelling, and that the definition is visible.";
    E0202: "defined twice",
        "Two functions in the same file have the same name.";
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
         exactly once, where it certainly has no value yet.";
    E0206: "module not imported",
        "A module was used without importing it. Nothing is imported\n\
         automatically, not even `io`: add `in io;` at the top of the file.";
    E0207: "read before assignment",
        "A binding declared without a value was read on a path where it may\n\
         not have been given one yet.\n\n\
         \x20   uint32 x;\n\
         \x20   if (ready) { x = 1; }\n\
         \x20   io::outln(x::text());   // error: no value if `ready` is false";

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
