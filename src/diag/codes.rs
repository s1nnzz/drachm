//! The error code table. `E` codes are errors, `W` codes are warnings; both
//! share one number space, grouped by the first two digits:
//!
//! - `00xx` syntax (lexer and parser)
//! - `01xx` types and conversions
//! - `02xx` names, declarations and bindings
//! - `03xx` ownership and borrowing (step 3)
//! - `05xx` constructors, enums, optionals and failures (step 5)
//! - `06xx` generics, interfaces, collections and lambdas (step 6)
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
    E0112: "no such operator",
        "An operator was used on a struct that has no operator method for it.\n\
         The left side leads, so `a + b` calls the `+` method of `a`'s struct:\n\n\
         \x20   struct Vector3 {\n\
         \x20   public:\n\
         \x20       float64 x; float64 y; float64 z;\n\
         \x20       Vector3 +(Vector3 o) { Vector3 r = self.x + o.x, self.y + o.y, self.z + o.z; return r; }\n\
         \x20   }\n\n\
         `==` and `!=` need no method: structs compare field by field (strings\n\
         by their text) unless they define `bool ==(T o)`.";
    E0113: "invalid operator method",
        "An operator method has the wrong shape. Operator methods are named by\n\
         one of `+ - * / % **` or `== != < <= > >=`, take exactly one parameter\n\
         (the right-hand side), and are never `static`. Comparisons return\n\
         `bool`, and `==` / `!=` take a value of the struct's own type.";
    E0114: "struct construction",
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

    E0308: "which parameter the result borrows from is ambiguous",
        "A function that returns a borrow gets its `lives` from its body when only\n\
         one parameter can be the source. When the result may borrow from several\n\
         parameters, depending on the path taken, say which with `lives`:\n\n\
         \x20   &string longest(&string a, &string b) lives a, b {\n\
         \x20       if (a::len() > b::len()) { return a; }\n\
         \x20       return b;\n\
         \x20   }\n\n\
         A method that may return a borrow of `self` or of a parameter names `self`\n\
         after `from`: `lives from self or key`.";
    E0309: "`lives` does not match the body",
        "A function's `lives` clause names whole parameters that hold a borrow,\n\
         and the result may borrow from those. Naming something that is not a\n\
         parameter holding a borrow is an error:\n\n\
         \x20   &string pick(&string a, uint32 n) lives n { ... }   // `n` holds no borrow\n\n\
         It is also an error when the body returns a borrow of a parameter the\n\
         clause does not name:\n\n\
         \x20   &string pick(&string a, &string b) lives a { return b; }   // add `b`";
    E0320: "a borrow stored where the caller cannot promise it",
        "A parameter that holds a borrow, such as `&mut &string a`, can only be given\n\
         borrows that last as long as what the caller put there. Storing a borrow of\n\
         another parameter into it is not allowed, because the caller cannot promise\n\
         the other parameter lives that long:\n\n\
         \x20   none keep(&mut &string a, &string b) {\n\
         \x20       a = b;                   // error\n\
         \x20   }\n\n\
         Store a copy of the value instead.";
    E0314: "struct borrows without `lives`",
        "A struct with a borrow field (`&string src;`) must say what it borrows\n\
         from, and `lives` must name exactly the borrow fields:\n\n\
         \x20   struct Parser lives src { &string src; uint32 pos; }";
    E0317: "`unsafe` without the file flag",
        "`unsafe { }`, `unsafe` functions, `*T` types and `address of` need the\n\
         file flag `//--!$unsafe: true` at the very top of the file.";
    E0321: "missing `&` or `&mut` at the call",
        "A parameter of type `&T` or `&mut T` is passed by writing the borrow at\n\
         the call, so the call says what the callee may do:\n\n\
         \x20   none grow(&mut string s) { ... }\n\
         \x20   grow(&mut xs);          // not `grow(xs)`";
    W0322: "`copy` of a value that copies anyway",
        "`copy` makes a deep copy, but numbers, bools, shared borrows and small\n\
         plain structs always copy when used, so `copy` changes nothing.";
    W0323: "`mut open` is redundant",
        "`mut` already allows everything `open` does, so `mut open` says the same\n\
         as `mut`.";
    E0326: "`lend` needs a `&mut` you hold",
        "`lend r` passes on a `&mut` borrow you hold and keeps it afterwards. The\n\
         operand must be a place of type `&mut T`; to borrow a plain value, write\n\
         `&mut x`.";
    E0327: "not a variable or field",
        "`take`, `copy` and `address of` work on a place: a variable or a field\n\
         of one. A call result or other temporary value has nothing to take out\n\
         of; use it directly.";
    E0328: "`drop` has the wrong shape",
        "A method named `drop` runs by itself when the value is destroyed, so it\n\
         must be written `none drop()`: not `static`, no parameters, not `unsafe`.";
    E0329: "borrow of a `&mut` you hold",
        "`&mut s` where `s` is already a `&mut` binding would borrow the borrow.\n\
         To pass the borrow on and keep it, write `lend s`.";
    E0330: "`&mut` where `&` is wanted",
        "A `&mut` binding passed bare where a `&T` is expected must be written\n\
         `&m`: an explicit shared borrow through the `&mut` you hold.";
    E0331: "calling `drop` yourself",
        "`drop` runs by itself when a value is destroyed. To end a value now,\n\
         write `destroy x;`.";

    E0324: "field moved without `take`",
        "Moving a field out of a struct you own leaves the struct without that\n\
         field, so it must be said: `take p.name` moves it out and leaves `p.name`\n\
         without a value until you give it one. To keep both, write `copy p.name`.";
    E0325: "`take` on a struct with `drop()`",
        "A struct with a `drop()` method must stay whole, because `drop` runs when\n\
         it is destroyed. Take a field only with a replacement in the same\n\
         statement:\n\n\
         \x20   string s = take f.path leaving \"\";";

    E0301: "used after it moved",
        "A value of a type that does not copy automatically (`string`, a big\n\
         struct, a `&mut` borrow) moves when it is assigned, passed or returned.\n\
         The old name cannot be used again until it is given a new value:\n\n\
         \x20   string a = \"x\";\n\
         \x20   string b = a;            // `a` moved\n\
         \x20   io::outln(a);            // error\n\n\
         To keep both, copy: `string b = copy a;`. Moving one field out with\n\
         `take p.name` leaves `p.name` without a value until it is assigned\n\
         again; `p` as a whole cannot be used meanwhile.";
    E0302: "cannot change while borrowed",
        "A value cannot be changed while a borrow of it is still going to be\n\
         used. The borrow ends after its last use, so use it first:\n\n\
         \x20   mut string name = \"Illy\";\n\
         \x20   &string first = &name;\n\
         \x20   name = \"Sam\";             // error: `first` is used below\n\
         \x20   io::outln(first);\n\n\
         Move `io::outln(first);` above the assignment, or do not hold the\n\
         borrow across it.";
    E0303: "two `&mut` at once",
        "Only one `&mut` borrow of a value can be in use at a time, so that\n\
         nothing else can change it behind your back:\n\n\
         \x20   &mut Player a = &mut p;\n\
         \x20   &mut Player b = &mut p;     // error: `a` is used below\n\
         \x20   heal(a);\n\n\
         Finish using the first borrow before taking the second.";
    E0304: "used while borrowed with `&mut`",
        "While a `&mut` borrow of a value is going to be used, the value itself\n\
         cannot be read or borrowed again, only the borrow can:\n\n\
         \x20   &mut Player a = &mut p;\n\
         \x20   io::outln(p.hp::text());    // error: `a` is used below\n\
         \x20   heal(a);\n\n\
         Read through the borrow (`a.hp`), or read before borrowing.";
    E0305: "moved while borrowed",
        "A value cannot be moved away while a borrow of it is still going to be\n\
         used:\n\n\
         \x20   string name = \"Illy\";\n\
         \x20   &string first = &name;\n\
         \x20   consume(name);              // error: `first` is used below\n\
         \x20   io::outln(first);\n\n\
         Finish with the borrow first, or move a copy: `consume(copy name)`.";
    E0306: "does not live long enough",
        "A value is dropped at the end of its block, or when its function\n\
         returns, but a borrow of it is still going to be used after that:\n\n\
         \x20   &string keep;\n\
         \x20   if (true) {\n\
         \x20       string local = \"x\";\n\
         \x20       keep = &local;\n\
         \x20   }                           // `local` is dropped here\n\
         \x20   io::outln(keep);            // error\n\n\
         Declare the value outside the block, so that it lives as long as the\n\
         borrow is used.";
    E0307: "returns a borrow of a local",
        "A function cannot return a borrow of something it owns: the value is\n\
         dropped when the function returns, and the borrow would point at\n\
         nothing. This covers its own locals, its by-value parameters and\n\
         temporaries:\n\n\
         \x20   &string shout() {\n\
         \x20       string s = \"hi\"::upper();\n\
         \x20       return &s;               // error\n\
         \x20   }\n\n\
         Return the value itself (`string shout()` and `return s;`), or borrow\n\
         from a parameter that is already a borrow.";
    E0310: "cannot move out of a borrow",
        "A function that only borrows a value does not own it, so it cannot\n\
         move anything out of it, `self` and raw pointers included. Return or\n\
         pass a copy instead:\n\n\
         \x20   string nameOf(&Player p) { return copy p.name; }";
    E0311: "cannot change through a `&` borrow",
        "A `&` borrow only reads what it points to, and that includes the `mut`\n\
         fields of a struct behind it. Changing it, lending it `&mut`, or calling\n\
         a method that changes `self` needs a `&mut` borrow:\n\n\
         \x20   none hurt(&mut Player p) { p.hp = p.hp - 1; }";
    E0312: "cannot lend `&mut` from an immutable binding",
        "A `&mut` borrow, and a method that changes `self`, need a binding that is\n\
         `mut` (it may also be reassigned) or `open` (its contents may change).\n\
         A temporary value has no name to make either, so it cannot be lent\n\
         `&mut` at all. A method that changes only `mut` fields is the exception:\n\n\
         \x20   Counter k = \"page\", 0;\n\
         \x20   k::hit();                 // fine if `hit` writes only `mut` fields\n\
         \x20   open string xs = \"a\";\n\
         \x20   grow(&mut xs);            // fine";
    E0313:"assigning part of a moved value",
        "A field cannot be given a value while the struct holding it has moved\n\
         as a whole. Give the whole struct a new value first:\n\n\
         \x20   consume(p);\n\
         \x20   p = ...;\n\
         \x20   p.hp = 5;";
    E0315: "cannot copy a value holding `&mut`",
        "A copy of a struct with a `&mut` field would be a second `&mut` to the\n\
         same value. Borrow the struct instead: `&Writer v = &w;`.";
    E0332: "assigning part of a value that has none yet",
        "A field cannot be given a value while the struct holding it has no\n\
         value at all yet. Give the whole struct a value first:\n\n\
         \x20   mut Player p;\n\
         \x20   p = ...;\n\
         \x20   p.hp = 5;";
    E0401: "units do not match",
        "Two values were combined, compared or assigned, and their units measure\n\
         different kinds of things: metres and seconds, say. Values add, subtract\n\
         and compare only when they are the same kind of unit (`10<km> + 100<m>`\n\
         is fine). To combine different kinds, multiply or divide: `dist / time`.";
    E0402: "unknown unit",
        "A unit was used that is not declared. Declare it with `unit NAME;` for a\n\
         new base unit, or `unit NAME = ...;` to give it a size in other units,\n\
         such as `unit km = 1000 m;`.";
    E0403: "bad unit declaration",
        "A `unit` declaration is not valid: a name declared twice, a unit defined\n\
         in terms of itself, a factor that is not a number greater than zero, an\n\
         `offset` after anything but a single unit, or a name that is not plain\n\
         ASCII letters, digits and `_`.";
    E0404: "a unit can only be raised to a constant whole power",
        "The unit of `side ** n` is the unit of `side` raised to `n`, so `n` must\n\
         be a whole number the compiler can see (`side ** 2`). Write `side * side`\n\
         or use a constant exponent.";
    E0405: "a unit cannot go here",
        "A unit in angle brackets (`float64<m>`) goes only on a number type, and a\n\
         unit's power (`m^2`) must stay within the range of a 16-bit exponent.";
    E0406: "bit operators cannot be used on a value with a unit",
        "`<<`, `>>`, `&`, `|`, `^` and `~` change a pattern of bits, not a measure.\n\
         For a value with a unit use arithmetic: `len * 2` instead of `len << 1`.";
    E0407: "cannot convert to this unit",
        "`x::in<unit>()` changes the unit of `x` to another unit of the same kind\n\
         (`d::in<km>()`), and `x::to<T>()` changes only the number type, so `T` is\n\
         a plain number type (`d::to<float32>()`). A point such as `degC` cannot\n\
         be turned into a `delta`.";
    E0408: "a plain number cannot stand for a measure",
        "A number without a unit was used where a measure is needed: `d + 5`,\n\
         `d < n`, or a plain number given to a variable with a unit. Write the\n\
         unit (`d + 5<m>`). Multiplying and dividing by plain numbers is fine, and\n\
         zero is unit-free in `+`, `-` and comparisons, except beside a point such\n\
         as `degC`.";
    E0409: "points and deltas cannot be mixed this way",
        "A unit with an offset, such as `degC`, measures a point on a scale, not an\n\
         amount. Subtract two points to get a `delta`, and add a `delta` to a point\n\
         to get a point. Two points cannot be added, and a point cannot be scaled\n\
         or negated.";

    E0333: "cannot write through a `*fixed` pointer",
        "A `*fixed T` pointer comes from `address of` an immutable binding, so it\n\
         can only be read. To write through a pointer, make the binding it\n\
         points to `mut` (or `open`) and take the address again:\n\n\
         \x20   mut uint32 n = 5;\n\
         \x20   unsafe { *uint32 p = address of n; *p = 6; }";
    E0334: "`*T` and `*fixed T` do not convert to each other",
        "A `*T` and a `*fixed T` are different kinds of pointer, and an assignment,\n\
         argument or comparison does not change one into the other. To change\n\
         the kind, say so inside an `unsafe { }` block:\n\n\
         \x20   unsafe { *fixed uint32 r = w::freeze(); }   // `*T` to `*fixed T`\n\
         \x20   unsafe { *uint32 w2 = r::thaw(); }          // `*fixed T` to `*T`";

    E0316: "an operation on a raw pointer needs `unsafe`",
        "Making, reading, writing, copying, comparing, converting, moving, passing\n\
         or returning a raw pointer, and calling an `unsafe` function, must be\n\
         inside an `unsafe { }` block. The body of an `unsafe` function is one\n\
         such block:\n\n\
         \x20   none read(*uint32 p) { unsafe { uint32 v = *p; } }\n\
         \x20   unsafe none bump(*uint32 p) { *p = *p + 1; }";

    E0520: "duplicate variant",
        "An enum has two variants with the same name. Each variant of an enum\n\
         needs a name of its own, because the name is how a value of that\n\
         variant is built and matched.\n\n\
         \x20   enum Shape { Circle(float64 r), Circle }     // error";
    E0521: "unknown variant",
        "The name after `Enum::` is not a variant of that enum. The message\n\
         lists the variants it does have.\n\n\
         \x20   Shape s = Shape::Triangle(1.0);               // error: no `Triangle`";
    E0522: "wrong number of values for a variant",
        "A variant is built with exactly one value for each value it declares,\n\
         in order: `Rect(float64 w, float64 h)` takes two.\n\n\
         \x20   Shape s = Rect(2.0);                          // error: needs 2";
    E0523: "cannot tell which enum a variant belongs to",
        "A bare variant name such as `Circle(1.0)` or `Empty` takes its enum\n\
         from the type the value is going to: a declaration with a written\n\
         type, an assignment, an argument or a `return`. Where nothing says,\n\
         write the enum: `Shape::Circle(1.0)`.";
    E0525: "pattern of another enum",
        "A `case` names something that is not a variant of the enum being\n\
         matched. The message lists the variants that exist.";
    E0527: "enum with values compared with `==`",
        "`==` and `!=` work on enums whose variants carry no values (`d == North`).\n\
         An enum with values is taken apart with `match`.\n\n\
         \x20   if (shape == Empty) { }                      // error when some variant has values";
    E0528: "binding that owns memory needs take or copy",
        "A payload that owns memory (a `string`, or a struct holding one) cannot\n\
         be bound by a plain `Type name` in a `case`: the pattern has to say\n\
         whether it moves the payload out or copies it.\n\n\
         \x20   case Text(take string body) { }   // moves it; the matched value is used up\n\
         \x20   case Text(copy string body) { }   // copies it; the matched value stays whole\n\n\
         Numbers, bools and Copy structs bind plainly: `case Circle(float64 r)`.";
    E0529: "inconsistent bindings in a case",
        "A name is bound twice in one pattern, or the patterns of one `case`\n\
         (`case A(float64 x), B(float64 y)`) do not bind the same names with\n\
         the same types, so the body could not know which one it has.";
    E0530: "`none` where no optional type is wanted",
        "`none` is the empty value of an optional type (`uint32? x = none;`).\n\
         Where no optional type is expected there is nothing for it to be empty\n\
         in, so the place needs an optional type.\n\n\
         \x20   uint32? x = none;                           // fine\n\
         \x20   uint32 y = none;                            // error: no optional type";
    E0531: "an optional used as its value",
        "A `uint32?` may be `none`, so it is not a `uint32`. Arithmetic, a\n\
         plain `uint32` argument or a plain declaration cannot take it.\n\
         Give the empty case a value with `maybe`, or test it with `has value`\n\
         and `is empty`.\n\n\
         \x20   uint32? age = 5;\n\
         \x20   uint32 n = age maybe 0;                     // fine\n\
         \x20   uint32 m = age;                             // error: may be `none`";
    E0532: "`maybe` or `has value` on a value that is not optional",
        "`maybe`, `has value` and `is empty` work on an optional value (`T?`)\n\
         only: they tell whether there is a value to take.\n\n\
         \x20   uint32 n = 5;\n\
         \x20   uint32 m = n maybe 0;                       // error: `n` is not optional";
    E0533: "`none` compared with `==`",
        "An optional is not compared with `none`. Ask whether it has a value\n\
         with `has value`, or whether it is empty with `is empty`.\n\n\
         \x20   if (age is empty) { }                       // instead of `age == none`\n\
         \x20   if (age has value) { }                      // instead of `age != none`";
    E0534: "the fallback of `maybe` is more than one value",
        "The fallback after `maybe` is a single value, or a parenthesised\n\
         expression. A binary operator after it would read as part of the\n\
         fallback, so the fallback needs parentheses to say what it is.\n\n\
         \x20   uint32 n = (age maybe 0) + 1;               // fine\n\
         \x20   uint32 m = age maybe (n * 2);               // fine\n\
         \x20   uint32 k = age maybe 0 + 1;                 // error";
    E0526: "wrong type in a pattern",
        "A binding in a variant pattern must name exactly the type of the\n\
         value it binds (no conversion is applied in patterns), and must\n\
         bind as many values as the variant has.\n\n\
         \x20   case Rect(string w, float64 h) { }            // error: `w` is a float64";

    E0501: "no constructor accepts these values",
        "A struct with constructors was built from values that none of them takes:\n\
         the count is wrong, a name matches no parameter, or the types do not\n\
         convert. The labels list every constructor's parameters.\n\n\
         \x20   Circle(float64 radius) { ... }\n\
         \x20   Circle c = \"five\";          // error: no constructor takes a string";
    E0502: "more than one constructor fits equally well",
        "Two constructors could take the values and neither is a better fit: for\n\
         example `(float64)` and `(uint32)` both take a `uint8` without losing\n\
         anything. Convert the value yourself so only one fits:\n\n\
         \x20   Port p = 5::to<uint16>();";
    E0503: "two constructors take the same types",
        "Two constructors of one struct have the same parameter types, so a call\n\
         could not tell them apart. Change the parameter types of one of them.";
    E0504: "constructor leaves a field without a value",
        "A constructor returns before every field of the struct has a value. Each\n\
         field must be given one on every path to the end of the constructor:\n\n\
         \x20   Player(string n) { self.name = n; }   // error: hp has no value";
    E0505: "fill leaves a field without a value",
        "A `fill` block ends before every field has a value. Give the field a value\n\
         in the block, or end with `fill partial` and complete the value later.";
    E0506: "fill of a value that already has one",
        "`fill` gives the fields of a value that has no value yet. A value that is\n\
         complete already cannot be filled; declare it without a value first\n\
         (`Player p;`), or change its fields with plain assignments if it is `mut`.";
    E0507: "value used before it is complete",
        "A struct being filled, or a constructor's `self`, can only have its fields\n\
         given values until every field has one. Reading it, borrowing it or calling\n\
         a method on it is an error. A field that is already set can be read with\n\
         `now`: `now p.hp * 2`.";
    E0508: "no such def",
        "A `def` name in `with NAME`, `from NAME` or `defaults NAME` names no def for\n\
         the struct. Defs are written `def NAME of STRUCT { field = value; }`.";
    E0509: "field or def given twice",
        "A field gets two values in one `def` or `fill`, or two defs have the same\n\
         name. Keep one of them.";
    E0510: "no default def",
        "`fill ... with defaults` needs the struct to name its default def, with\n\
         `struct Player defaults PlayerStart { ... }` or `struct default Player { ... }`.";
    E0540: "not a failure type",
        "Only a `failure` can be listed after `fails`. The name is either not a\n\
         type at all, or it is a struct or an enum.\n\n\
         \x20   failure ParseError { Empty \"nothing to parse\" }\n\
         \x20   uint32 parse(string s) fails ParseError { ... }";
    E0541: "failure not in the list",
        "`fail X;` names a failure variant that none of the failure types in the\n\
         function's `fails` list has. Add the failure type to the list, or fail\n\
         with one of the variants it does have.";
    E0542: "fail in a function that does not say fails",
        "`fail` ends a function with a failure, so the function has to say so:\n\
         write `fails ParseError` after the parameters.\n\n\
         \x20   uint32 parse(string s) fails ParseError { fail Empty; }";
    E0543: "ambiguous failure variant",
        "Two of the failure types in the `fails` list have a variant with this\n\
         name. Write the failure type too: `fail ParseError::Empty;`.";
    E0544: "a call that can fail is not handled",
        "A call to a function that can fail has to say what happens when it\n\
         does. Fall back to a value with `maybe`, or pass the failure up with\n\
         `fails` (the caller must then list the failure type itself).\n\n\
         \x20   uint32 n = parse(line) maybe 0;\n\
         \x20   uint32 m = parse(line) fails;";
    E0545: "handler for a failure the call cannot raise",
        "A `fails with (Type e)` handler names a failure that the call cannot\n\
         fail with. Handle one of the failures the function lists after `fails`.";
    E0546: "failure handled twice",
        "Each failure type gets one handler after a call.";
    E0547: "handler gives no value",
        "A handler for a call that returns a value has to give one with\n\
         `maybe v;`, or leave the function (`return`, `fail`) or the loop\n\
         (`break`, `continue`). Ignoring a failure (`fails with none`) is only\n\
         for calls that return nothing.\n\n\
         \x20   uint32 n = parse(line) fails with (ParseError e) => { maybe 0; };";
    E0548: "failure not declared by the caller",
        "`call fails` passes the failures of the call up to the caller, so the\n\
         caller has to list every one of them after its own `fails`.";
    E0549: "because needs a failure",
        "`fail X because e;` records the failure `e` as the cause of `X`. The value\n\
         after `because` has to be a failure value.";
    E0550: "maybe outside a handler",
        "`maybe v;` gives the value of the call whose failure a handler is\n\
         dealing with, so it can only be written inside a `fails with` block.";
    E0551: "unknown value in a failure message",
        "A failure message can use the values of its variant with `${name}`. The\n\
         name is not one of the variant's values.";
    E0552: "exit code out of range",
        "The exit code of a failure is a whole number from 0 to 255.";
    E0553: "catch-all handler out of place",
        "`otherwise (any e)` and `fails with (any e)` handle every failure the\n\
         handlers before them did not, so they come last, and there is only one.";
    E0554: "failure list problem",
        "A failure type is listed twice after `fails`, or a `failure` has no\n\
         variants at all.";
    E0601: "cannot tell what a type parameter is",
        "A generic function's type parameter (`T`) is worked out from the\n\
         arguments of the call. When no argument shows it, or the call only\n\
         passes values with no type of their own that leave it open, write it\n\
         out between `<` and `>`.\n\n\
         \x20   T make<T>() { ... }\n\
         \x20   make<uint32>();                             // fine\n\
         \x20   make();                                     // error: what is `T`?";
    E0602: "wrong number of type arguments",
        "A type is given type arguments, but it takes none, or a different\n\
         number. `Point` is not generic, so `Point<uint32>` is an error; a\n\
         `Stack<T>` takes exactly one.";
    E0603: "type arguments disagree",
        "One type parameter is used for several arguments, and they do not have\n\
         the same type. Convert one of them, or write the type out so the others\n\
         are converted to it.\n\n\
         \x20   T max<T>(T a, T b) { ... }\n\
         \x20   max(a_u8, a_string);                        // error: `T` is two types\n\
         \x20   max<uint32>(a, b);                          // fine";
    E0604: "type arguments missing",
        "A generic function or type was named without its type arguments where\n\
         a type or a value is needed. A generic function used as a value, or a\n\
         generic type written alone, has to say what its type parameters are.";
    E0605: "bad type parameter list",
        "A type parameter is declared twice in the same list, or has the name of\n\
         a struct, enum or unit. A type parameter takes no other name's place.";
    E0608: "a generic keeps instantiating itself",
        "A generic function calls itself with a bigger type each time\n\
         (`grow<uint32>` calls `grow<uint32?>`, which calls `grow<uint32??>`, ...),\n\
         so the compiler would never stop making copies. Make the recursive call\n\
         use the same type. The same happens to a generic struct that holds an\n\
         instance of itself with a bigger type (`struct Bad<T> { Bad<Cell<T>> x; }`).";
    E0609: "a unit parameter used as a type",
        "`unit U` declares a unit, so it can only be used inside the `<>` of a\n\
         number type: `float64<U>`. A type parameter would be written `<T>`.";
    E0606: "beyond the constraint",
        "Inside a generic function or struct whose type parameter has a\n\
         constraint (`T with Shape`), a value of type `T` can only do what the\n\
         interface promises: call its methods, read its required fields, call\n\
         its `static` methods as `T::zero()`. Anything else, such as another\n\
         method or an operator the interface does not declare, is an error in\n\
         the definition, even if every type used so far happens to allow it.\n\n\
         \x20   float64 total<T with Shape>(T a) { return a::perimeter(); }   // error";
    E0607: "constraint not met",
        "A generic function or struct was used with a type that does not\n\
         implement the interface its type parameter asks for (`T with Shape`).\n\
         A struct implements an interface by naming it: `struct Triangle with\n\
         Shape`, and giving it the methods the interface lists.";
    E0610: "interface method missing",
        "A struct says `with Shape`, but has no public method for one of the\n\
         methods `Shape` lists. A method under `private:` does not count: an\n\
         interface's methods are public.";
    E0611: "interface method differs",
        "A struct's method has the name of an interface method, but its\n\
         parameters, its result or its `static` do not match. `Self` in the\n\
         interface stands for the struct that implements it.";
    E0612: "changes more than the interface allows",
        "A struct method changes `self`, but the interface method it\n\
         implements does not say `changes self`, so callers holding a shared\n\
         borrow would be promised something the method breaks. Either add\n\
         `changes self` to the interface method, or make the struct method stop\n\
         changing `self`. (A method that writes only `mut` fields counts as\n\
         changing.)";
    E0613: "changes self out of place",
        "`changes self` is only written after an interface method's\n\
         parameters. A struct's methods and a function's mode are worked out\n\
         from their bodies.";
    E0614: "required field missing",
        "An interface lists a field (`float64 x;`), so a struct that implements\n\
         it needs a `public` field of that name and type.";
    E0615: "interfaces clash",
        "An interface extends itself (directly or through others), or extends\n\
         two interfaces that have a member of the same name with different\n\
         signatures, or lists the same member twice.";
    E0616: "Self out of place",
        "`Self` stands for the implementing type, so it only means something\n\
         inside an interface. In a struct, write the struct's name.";
    E0617: "interface used as a value type",
        "An interface is only ever borrowed: a value, a field, a list item or a\n\
         type argument cannot be `Shape` itself, because it could be any struct\n\
         that implements it. Write `&Shape`, or use an enum for a closed set of\n\
         choices.";
    E0620: "index outside an array",
        "An array has a fixed length, so an index written as a number can be\n\
         checked while the program is compiled. Valid indexes run from 0 to one\n\
         less than the length, or from -1 (the last item) back to minus the\n\
         length.\n\n\
         \x20   uint8[4] rgba = [255, 0, 0, 255];\n\
         \x20   uint8 a = rgba[5];                          // error: only 0 to 3";
    E0622: "an item cannot be moved out of a list by index",
        "Reading `names[0]` where the item owns memory (a `string`, a struct\n\
         holding one) would leave a hole in the list. Borrow the item with\n\
         `&names[0]`, copy it with `copy names[0]`, or call a method on it.";
    E0623: "a list literal mixes types",
        "The items of `[a, b, c]` must all have one type. When the list is not\n\
         given a type, the first item decides it.";
    E0624: "an empty list has no type",
        "`[]` holds nothing to take a type from, so something has to say what the\n\
         list holds: `string[?] xs = [];`.";
    E0625: "wrong number of items for an array",
        "An array has a fixed length, and its literal must give exactly that many\n\
         items: `uint8[4] a = [1, 2, 3, 4];`.";
    E0626: "this type cannot be a map key",
        "A map finds an entry by hashing its key, so a key must be hashable: a whole\n\
         number, a float (compared by its bits), `bool` or `string`. Lists, maps,\n\
         borrows and pointers cannot be keys.";
    E0630: "the loop variable's type is not the items' type",
        "A loop over a list or array gives each item to the variable, so the\n\
         variable must have the items' type: `for (uint32 n of xs)` over a\n\
         `uint32[?]`. To look at each item write `&uint32 n`; no conversion\n\
         happens inside a `for`.";
    W0600: "generic never used",
        "A generic function, struct or enum is only checked when something uses\n\
         it with particular types, so one nothing uses is not checked at all.";
    E0901: "not supported yet",
        "This is part of Drachm's design, but this version of the compiler does\n\
         not implement it yet.";

    E0318: "invalid file flag",
        "A `//--!$` line at the top of a file sets a file flag. Flags are read\n\
         before the code, so they come first: above the first line of code,\n\
         after an optional `#!` line, with no blank line in between.\n\n\
         \x20   //--!$unsafe: true\n\n\
         A flag is `key: value`. `unsafe` takes `true` or `false`. A line that\n\
         is not `key: value`, a value the key does not accept, or a flag below\n\
         the top block is an error.";
    W0319: "unknown file flag",
        "A `//--!$` line names a key this compiler does not know. The line is\n\
         ignored, so the file still builds, but a misspelt key does nothing:\n\
         `//--!$unsfe: true` is not `unsafe`. New keys come with later versions\n\
         of the language, so older compilers warn instead of failing.";
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
