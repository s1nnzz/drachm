//! The error code table. `E` codes are errors, `W` codes are warnings; both
//! share one number space, grouped by the first two digits:
//!
//! - `00xx` syntax (lexer and parser)
//! - `01xx` types and conversions
//! - `02xx` names, declarations and bindings
//! - `03xx` ownership and borrowing (step 3)
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
         for example `#` or `@` outside a comment.";
    E0002: "number literal out of range",
        "An integer literal is too large to be represented.";
    E0003: "unexpected token",
        "The parser needed one thing and found another: a missing `;`, a missing\n\
         `)`, a statement that starts with something that cannot start one, and\n\
         so on. The label shows where, and the message says what was expected.";
    E0004: "unclosed block",
        "A `{` was opened and the file (or the next function) started before its\n\
         matching `}`. The label points at the `{` that was never closed.";
    E0005: "invalid assignment target",
        "The left side of `=` must be something that can be assigned to, such as\n\
         a variable. `1 + 2 = x;` and `f() = 3;` are not assignments.";

    E0101: "type mismatch",
        "A value of one type was used where another type is required.\n\n\
         \x20   let x: i64 = true;   // expected i64, found bool\n\n\
         Change the value, or change the declared type.";
    E0103: "wrong number of arguments",
        "A function was called with more or fewer arguments than it has\n\
         parameters.";
    E0104: "no value here",
        "An expression that produces no value was used where a value is needed:\n\
         stored in a variable, compared, or printed.";
    E0105: "missing return",
        "A function that declares a return type must return a value on every\n\
         path. Either a path reaches the end of the function without `return`,\n\
         or a `return;` gives no value.";

    E0201: "unknown name",
        "A variable, function or type was used that is not defined in this scope.\n\
         Check the spelling, and that the definition is visible from here.";
    E0202: "defined twice",
        "Two functions in the same file have the same name.";
    E0203: "builtin redefined",
        "A function has the name of a builtin, which cannot be replaced.";
    E0204: "missing or invalid `main`",
        "A program needs exactly one `main` function that takes no parameters\n\
         and returns either nothing or an exit code.";
    E0205: "assignment to immutable binding",
        "A binding that was not declared changeable was assigned to after its\n\
         declaration. Declare it changeable if it needs to change.";
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
