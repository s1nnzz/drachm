# WP0a: syntax, types and type checking for borrows

Implementer: **needs a strong model** (steps 5 and 6 are semantic work in
`typeck.rs`; steps 1-4 are mechanical and can be done first and committed
separately). Runs in parallel with WP0b and WP1 phase A. Read first:
`step3-ownership.md` 1.4 (R1-R18), 2.2, 2.4, 3.9, section 4 (texts), section 5
("Still open" list gives the E0901 messages), Appendix A.

## Goal

The language front end accepts the new syntax, types it, and lowers it to HIR.
No ownership *checking* happens here (that is MIR, WP2+): `&mut` of an immutable
binding, use after move, lifetimes are all accepted. Programs that use borrows
type-check (`drachm check`) but cannot be built yet. All existing tests pass
with unchanged output.

## Files you own

`src/lexer.rs`, `src/parser.rs`, `src/ast.rs` (except the one `unsafe_allowed`
line, below), `src/types.rs`, `src/hir.rs`, `src/typeck.rs`,
`src/typeck/{control,strings}.rs`, `src/diag/codes.rs`, and minimal arms in
`src/codegen.rs` / `src/cgen.rs` (below). Plus `src/lib.rs`'s **`build()` body
only**.

Do not touch: `src/mir/` (WP1), `src/flags.rs`, `lib.rs::check`, `main.rs`,
`tests/run.rs` (new test *files* are fine).

Conflict rules with the parallel packages:
- `ast::Program` gets the field WP0b adds, with identical text:
  `/// The file flag ... ` `pub unsafe_allowed: bool,` (see `wp0b.md`), and
  `parser::parse` builds it as `false`. typeck reads `prog.unsafe_allowed`.
- `codes.rs`: insert your codes after the `E0213` entry (before `E0901`); WP0b
  appends after `E0901`.
- WP1 reads `hir.rs` through two helper functions only (`place`, `expr_span`
  in `src/mir/build.rs`). If you change `Place` or `Expr`, WP1 fixes those two
  functions after merging. **WP0a merges first.**

## Steps (each ends with `cargo test` green; commit after each)

### 1. Keywords (mechanical)
Add `take` and `lend` to `lexer::KEYWORDS` (SPEC §1: always reserved). `by`,
`leaving`, `address`, `stays`, `changes`, `asm` stay ordinary identifiers; the
parser recognises them by position (`Tok::Ident("address")` followed by
`Kw("of")`; `leaving` right after a `take` place; `stays` right after a struct
name or `lives` list). `drop` is a reserved method *name*, checked in typeck.
No existing test uses these words as names (checked).

### 2. AST and parser
- `ast::TypeName { name, span, mods: Vec<(TypeMod, Span)> }`, `enum TypeMod
  { Ref { mutable: bool }, Ptr }`, outermost first (`&mut string` = `[Ref
  mutable]`, `*uint32` = `[Ptr]`). `&` directly followed by `&`/`&&` or `*`
  inside a type is `E0901` "borrowing a borrow is not supported yet". The lexer
  makes `&&` one token and `**` another: handle both in `ty()`.
- `Param` and `Stmt::Let` get `open: bool` next to `mutable`. Both written is
  W0323 (typeck). Allowed in parameters (`open string s`).
- `Func`: `is_unsafe: Option<Span>` (the word `unsafe` before the return type;
  extend `at_item_start`), `lives: Vec<(String, Span)>`, `lives_span`. Clause
  loop after `)` accepting `lives a, b` (and `fails`: if present, E0901 "`fails`
  is not supported yet"), any order, each once (R14).
- `Struct`: `lives: Vec<(String, Span)>`, `stays: Option<Span>`, clause loop
  after the name accepting `lives`, `stays`, and `with` (keep the existing E0901
  for `with`), any order, each once. Replace the current immediate-E0901 on
  `with`.
- `Block` becomes `struct Block { stmts: Vec<Stmt>, end: Span }` (`end` = the
  `}`), implementing `Deref<Target = Vec<Stmt>>` and `IntoIterator for &Block`
  so most users compile unchanged; fix the rest. `Case.body`, `If`, loops and
  `Func.body` all use it.
- New `ExprKind`: `Borrow { mutable, operand }` (prefix `&e`, `&mut e`; `&&`
  token in prefix position is `E0901` as above), `Deref(Box<Expr>)` (prefix
  `*e`), `Copy(Box<Expr>)` (`copy e`), `Take { place, leaving: Option<Box<Expr>> }`
  (`take e [leaving e2]`), `Lend(Box<Expr>)`, `AddressOf(Box<Expr>)`
  (`address of e`), `NoneValue` (`none` in expression position; today `none` is
  only a type). These are all prefix operators at `unary()` precedence.
- `Stmt::Unsafe { body: Block, span }`.
- `Stmt::Assign.target` may now be any place expression: `name`, `self`,
  `a.b.c`, `*p`, `*p.f`. Statement start detection (`at_binding`, parser.rs
  ~287): a statement starting with `&` is a binding; starting with `*` is a
  binding when the next two tokens are `Ident Ident` (`*uint32 p = ...`) and an
  assignment otherwise (`*p = 7;`, `*p.f = 7;`); `mut`/`open` may precede a type
  that starts with `&` or `*`.
- Unit tests in `parser.rs` for every form above, including `mut &string r =
  &x;`, `open string s;`, `*p = 7;`, `*uint32 p = address of n;`, `take p.name
  leaving "x"`, `unsafe uint32 f() { }`, `struct S stays lives a { }` and
  `struct S lives a stays { }`.

### 3. Types
`types::Type` gets `Ref { mutable, to: &'static Type }` and `Ptr { to: &'static
Type }` with `Type::borrow(mutable, to)` / `Type::ptr(to)` interning through a
`static Mutex<HashSet<&'static Type>>` (same trick as `types::intern`). `Display`:
`&T`, `&mut T`, `*T`. Add arms to every exhaustive match: `hir::scalar_size`
(8 bytes), `codegen::cl_type` (pointer type), `cgen::c_type` (`T *`).
`Type::is_borrow()`, `is_ptr()`, `target()` helpers.

### 4. HIR (see step3 2.2)
- `Expr { kind, ty, span }` with `Expr::new(kind, ty)` (dummy span
  `Span::new(0, 0)`); rewrite the ~63 `hir::Expr { kind, ty }` sites to
  `Expr::new` mechanically, then set real spans where `Expr`s are made from an
  `ast::Expr` (locals, fields, calls and arguments, borrows, assignments' values,
  returns).
- `Place { local, proj: Vec<HirProj>, span }`, `enum HirProj { Field(usize),
  Deref }` replacing `fields`; update `codegen.rs:354` and `cgen.rs:222,233`
  (they iterate fields; a `Deref` there is `unreachable!` until WP4).
- `Local { name, ty, span, mutability: Mutability, is_param }`; `enum Mutability
  { Fixed, Mut, Open }` (`open` is parsed and stored, treated as `Mut` for
  checks until WP5: leave a `// WP5` comment at `check_assignable`).
- `Branch.end`, `Loop.end` (closing `}` span), `Func { name_span, span,
  body_end, is_unsafe, lives: Option<Vec<usize>> }` (`lives` = declared
  parameter indexes).
- `Stmt::Decl(LocalId)` emitted by typeck at each user declaration, before its
  initializer's `Assign`; `Stmt::Unsafe(Block)`. Backends: `Decl` ignored,
  `Unsafe(b)` runs `b`.
- New `ExprKind`: `Borrow { mutable: bool, place: Box<Expr> }`, `Deref(Box<Expr>)`,
  `DeepCopy(Box<Expr>)`, `Take { place: Place, leaving: Option<Box<Expr>> }`,
  `Lend(Box<Expr>)`, `AddressOf(Box<Expr>)`, `NullPtr`. Backends: `unreachable!()`.
- `FieldDef.mutable`; `StructDef { copy: bool, stays: bool, has_drop:
  Option<FuncId>, lives: Vec<usize> }`. `copy` is computed next to `lay_out`
  (typeck.rs:541): not `stays`, no `drop`, every field Copy (numbers, bool, `&T`,
  Copy structs; not `string`, `&mut`, `*T`), `size <= 32` (padding included).
- `lib::build`: if the program contains any new node or any `Ref`/`Ptr` type,
  return `Err("borrows and pointers cannot be built yet (step 3 is in progress)")`
  before codegen. Put a `uses_borrows: bool` on `hir::Program`, set by typeck.

### 5. Name and type resolution
- `Globals::resolve` handles `mods`; `*T` or `unsafe` anywhere with
  `!prog.unsafe_allowed` is **E0317** at the keyword/type (once per site; text in
  section 4). `&T` of a type that is already a borrow: E0901 as above.
- Struct declarations: `lives` names must be exactly the borrow fields (E0314,
  both variants); a borrow field type with 2+ slots or a struct-with-`lives`
  type is E0901 (Still open #7); borrow field without any `lives` is E0314 (text
  in section 4). Fill `StructDef.lives`. A method named `drop` must be `none
  drop()`, non-static, no params, not `unsafe`, else E0328; set `has_drop`.
- Function `lives a, b`: each name must be a parameter whose type holds a borrow,
  else E0309 (the "which is not a parameter holding a borrow" variant); store
  indexes in `Func.lives`.
- `W0323` for `mut open`.

### 6. Expression typing (the semantic core)
Implement R4/R18 in one place: a `Checker::read(e) -> hir::Expr` that inserts
`ExprKind::Deref` when `e.ty` is a borrow and the context wants the target, and
use it everywhere an operand, condition, field access (`.`), method receiver
(`::`), builtin argument, `Text`/`Join` part, `Convert`, init/assign/return of a
non-borrow type, or `copy` operand is checked. Contexts that want a borrow keep it
(declaration of a borrow type, borrow parameter/return/field, assignment to a
borrow, `lend`).

| Source | Typing / lowering | Errors |
|---|---|---|
| `&e`, `&mut e` | `e` a place (local, field chain, or a borrow's target) or any value (a temporary). Type `&T` / `&mut T`, `T` = type of `e` | `e` already a borrow: E0901 #1 |
| `lend e` | `e` a place of type `&mut T` → `Lend(e)`, type `&mut T` | non-place or non-`&mut`: E0326; `e: &T`: E0901 #2 |
| call/init/return of a `&T` / `&mut T` slot | the argument must be a borrow value of the right kind (`&e`, `lend e`, a borrow-typed place; a `&mut` binding passed bare is allowed and is a move later) | a plain `T` value: **E0321**; `&mut` where `&` wanted: E0901 #3; `&` where `&mut` wanted: E0101 mismatch |
| method receiver / builtin operand | implicit, no `&` written (R2, R3) | |
| `x = rhs` with `x: &T` / `&mut T` | `rhs` a borrow → re-point (`Assign(x, rhs)`); otherwise write through (`Assign(x.Deref, rhs)`); `+=`, `++` always through | |
| `*e` | on `*T`: `Deref`, type `T`; on a borrow: E0901 #4 | |
| `copy e` | `e` a place (else E0327); borrow → `DeepCopy(Deref e)`, type `T`; else `DeepCopy(e)`; **W0322** if the result type is Copy (R11) | |
| `take e [leaving v]` | `e` a field chain of an owned local (else E0327); through a borrow, of a Copy field, a whole local, or `leaving` through a borrow: E0901 #5. `Take { place, leaving }` | |
| `address of e` | `e` a place → `*T`; needs the flag (E0317) | non-place: E0327 |
| `none` | only where a `*T` is expected or beside a `*T` in `==` / `!=` → `NullPtr` | else E0901 #10 |
| `x::drop()` | | E0901 #6 |
| `unsafe { }` | `Stmt::Unsafe` | no flag: E0317 |
| `unsafe` fn declaration | `Func.is_unsafe` | no flag: E0317 |

`==` / `!=` on two borrows compares targets (R4). E0321/E0326/E0327/W0322/W0323/
E0314/E0317/E0328/E0309 texts are in section 4; give each a `codes.rs` entry
with an `explain`, in the style of the existing ones. E0901 messages are the
quoted strings in "Still open".

## Tests to add (fail tests need a `.err`; match on code and message)

1. `tests/fail/own_syntax.drh`
   ```
   in io;
   struct Parser { &string src; }
   none grow(&mut uint32 n) { }
   none main() {
       uint32 b = 2;
       grow(b);
   }
   ```
   `.err`: ``error[E0314]: `Parser` holds a borrow``,
   ``error[E0321]: `grow` takes `&mut uint32` here``.
2. `tests/fail/own_borrow_calls.drh`
   ```
   struct Player { string name; }
   none show(&Player p) { }
   none grow(&mut string s) { }
   none main() {
       Player p = "a";
       mut string xs = "b";
       show(p);
       grow(xs);
       show(&mut p);
   }
   ```
   `.err`: ``error[E0321]: `show` takes `&Player` here``, ``error[E0321]: `grow`
   takes `&mut string` here``, ``error[E0901]: using a `&mut` where a `&` is
   expected is not supported yet``.
3. `tests/fail/own_borrow_of_borrow.drh`: `none main() { string s = "a"; &string
   r = &s; &string q = &r; }` → ``error[E0901]: borrowing a borrow is not
   supported yet``.
4. `tests/fail/own_struct_lives.drh`: `struct Pair lives a { &string a; &string b;
   } struct Q lives x { uint32 n; } none main() { }` → ``field `b` is a borrow,
   but `lives` does not name it``, ``names nothing `Q` borrows``.
5. `tests/fail/own_forms.drh`:
   ```
   string make() { return "a"; }
   none main() {
       uint32 n = 1;
       string s = take make();
       string t = copy make();
       uint32 m = lend n;
   }
   ```
   → ``error[E0327]: `take` needs a field of a variable``, ``error[E0327]: `copy`
   needs a variable or a field``, ``error[E0326]: `lend` passes on a `&mut` borrow
   you hold, and `n` is not one``.
6. `tests/fail/own_drop_shape.drh`: `struct S { uint32 n; string drop(uint32 how) {
   return "x"; } } none main() { }` → ``error[E0328]: `drop` must be written
   `none drop()` ``.
7. `tests/fail/own_unsafe_no_flag.drh`: `none main() { unsafe { } }` plus a
   function `none f(*uint32 p) { }` → ``error[E0317]: `unsafe` is not enabled in
   this file`` (twice).
8. `tests/fail/own_lives_name.drh`: `&string pick(&string a, uint32 n) lives n {
   return a; } none main() { }` → ``error[E0309]``, ``names `n`, which is not a
   parameter holding a borrow``.
9. `tests/run/own_open_warn.drh` (`.out` = `3`, `.warn` = ``warning[W0323]``):
   `in io; none main() { mut open uint32 a = 1; open uint32 b = 2;
   io::outln((a + b)::text()); }`.
10. A type-check-only success: `tests/fail` cannot hold it, so add a Rust test in
    `tests/` (new file `tests/borrows.rs`) that calls `drachm::check` on a source
    using `&`, `&mut`, `lend`, `copy r`, `take p.f leaving v`, `*p` in a file with
    `//--!$unsafe: true` and `address of`, asserts no errors, and that
    `drachm::build` returns the "cannot be built yet" error.

## Done when

`cargo test` green; the nine new golden tests and the Rust test pass;
`optimised_output_matches` unchanged (backends only got dead arms);
`drachm check` on each of the SPEC §6 borrow examples type-checks.

## Pitfalls

- Prefix `&` versus binary `&`: prefix only where an operand is expected. The
  bitwise-grouping check (`E0006`, parser.rs `check_grouping`) must not fire on a
  prefix `&`.
- `&mut x`: `mut` is a keyword, so the parser sees `&` then `Kw("mut")`.
- `mut &string r = &x;` (mut before a borrow type) and `&string r = &x;` as a
  statement start; `*p = 1;` versus `*uint32 p = ...;` (look ahead two tokens).
- Do not make `take`/`lend` function names valid anywhere; fix the two test
  programs in the step3 spec if you copy them (`consume`, not `take`).
- Struct `copy` must exist before function bodies are checked (`copy e` warns by
  it); compute it in the same pass as `owns`/layout.
- Every `Expr::new` with a dummy span is a bug if the expression can appear in an
  ownership error; at minimum the ones listed in step 4 need real spans.
- Keep the legacy ownership nodes (`Copy`, `Move`, `Temp`, `Scoped`, `Drop`) as
  they are: the old backends still need them.
