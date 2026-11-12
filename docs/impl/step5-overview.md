# Step 5: constructors, enums, optionals, failures (overview)

This file is the plan for step 5. The packages have their own briefs; read this
file first, then your brief:

| Package | Brief | Implementer | What |
|---|---|---|---|
| S4R | `step4-rest.md` | standard | `match` on text with globs, `for` over characters |
| S5A | `step5-A-enums.md` | **strong** (A.1), standard (A.2) | enums with data, `match` with bindings and exhaustiveness (A.1); `Optional<T>`, `T?`, `none`, `maybe` on optionals (A.2) |
| S5B | `step5-B-constructors.md` | standard | constructors and overloads, `Type x = a, b;`, `new Type of (...)`, the automatic constructor (B.1); `fill`, `fill partial`, `def`, `defaults` (B.2) |
| S5C | `step5-C-failures.md` | **strong** | `failure` declarations, `fails`, `fail X;`, bare `fails`, `maybe`, the outcome lowering, `unhandled_fail`, `main` and exit codes |
| S5D | `step5-D-handlers.md` | standard | `fails with` chains, `otherwise (any e)`, `because` chains, `${e}`, failing `::to<T>()` (replaces step 4.6's exit-101), failing constructors |
| S5E | `step5-E-fast-abi.md` | **strong** | optional: return outcomes in registers; benchmark |

Nothing below changes the language. Where the owner decided something it is
quoted from `owner-answers.md` or SPEC; where nothing was decided there is an
**interim** choice that is conservative (an E0901 "not supported yet" or an
error that can later be relaxed) and the point is a numbered language question
at the end (Q1 to Q18). **Do not decide a language question yourself**; if your
work needs one answered, stop at the interim and report.

Every brief says, and every implementer must follow (CLAUDE.md "Keeping the
docs site current"): the site is updated **in the same commit** as the feature
becomes usable, and only documents what the compiler accepts today. Badges are
removed only for what builds. Commits are plain human messages with no
attribution lines. Do not push. The machine is shared with the owner: run
`cargo test` once per commit, not in a loop, use `nice -n 19` for benchmarks,
and never run two compiling jobs yourself.

## 1. What exists today (the state your work starts from)

- Parser (`src/parser.rs`): `fails` after a signature is E0901 (line ~388),
  constructors are E0901 (line ~502), `enum`/`failure` are not parsed, the
  keywords `enum failure fails fail maybe because otherwise any exit new none
  of` are reserved in `src/lexer.rs`. `fill`, `def`, `defaults`, `partial`,
  `default` are **not** reserved: make them context words, like `unit`
  (Q7 of the units work): special only at the start of a statement or top-level
  item where the next token makes the form unambiguous; everywhere else ordinary
  names. Add a parser test for `uint32 fill = 3;` and `fill(3)`.
- Types (`src/types.rs`): no enum type. `Type::Struct { id, name }`.
  Optional `?` is a symbol token but not parsed in types.
- Typeck (`src/typeck.rs`, `src/typeck/{control,strings,units}.rs`):
  `construct` builds the field-order shorthand; `init` handles `Init::Fields`;
  `none` as a value is E0901 (lines ~2132, 2328, 2340); `ParseText` is built in
  `typeck/strings.rs` (`parse_text`) and panics at run time on bad text.
- HIR (`src/hir.rs`): one `Func.ret`, structs only, `Stmt::If/Loop/Jump/...`.
  `match` is an `If` chain with value tests.
- MIR (`src/mir/*`): `Rvalue::Aggregate(StructId, ..)` for structs, `Proj::Field`
  and `Proj::Deref`, `TerminatorKind::Switch` is **defined, printed, validated,
  and never emitted**; both backends `unreachable!()` on it. Calls return
  `dest: Option<Place>`; a struct result comes back through a hidden first
  pointer parameter (`in_memory(ret)` in `codegen_common.rs`), a scalar in a
  register. Drop elaboration works on move paths made of `Field` projections
  (`mir/moves.rs`), glue is generated per struct (`drops::drop_glue_structs`,
  `copy_glue_structs`).
- Backends: Cranelift (`codegen_mir.rs`) and C (`cgen_mir.rs`) read the same
  MIR. Plain builds must behave exactly like `-O` builds; `tests/run.rs` runs
  every case both ways, leak-checked.
- Runtime: `runtime/runtime.c` (panic exit 101, output, input), `runtime/strings*`.

## 2. Design

### 2.1 Types and layout

- `Type::Enum { id, name }` (Copy, interned like `Struct`). `hir::Program.enums:
  Vec<EnumDef>`:

  ```rust
  pub struct EnumDef {
      pub name: String,
      pub variants: Vec<VariantDef>,       // declaration order; tag = index
      pub kind: EnumKind,
      pub tag_size: u32,                   // 1, 2 or 4 bytes: smallest that fits
      pub size: u32, pub align: u32,
      pub owns: bool,                      // some payload (or the cause) owns memory
      pub copy: bool,                      // all payloads Copy, <= 32 bytes, no cause
      pub has_cause: bool,                 // failure types only (2.4)
  }
  pub struct VariantDef { pub name: String, pub fields: Vec<FieldDef>, /* offsets inside the enum */ pub name_span: Span }
  pub enum EnumKind {
      User,
      Optional(Type),                      // `T?`: variants none (tag 0), Some(T value) (tag 1)
      Outcome { ok: Type, fail: Type },    // 2.3: variants Ok(T) (tag 0, no field when T is none), Fail(F) (tag 1)
      FailSet(Vec<EnumId>),                // 2.3: one variant per failure type
      Failure,                             // a `failure` declaration, with messages (2.4)
  }
  ```
- Layout: tag at offset 0; the cause pointer (failure types) at the next
  8-aligned offset; the payload area after it, at the largest alignment any
  variant needs; each variant's fields laid out C-style from the payload start
  (`hir::layout`); the size is the largest variant rounded up to the enum's
  alignment. All enums are "in memory" types for the ABI (like structs).
- `Program::size_align`, `owns`, `scalar_size`, the `struct_id()` users (about
  60 uses, `grep -n "struct_id()\|Type::Struct" src`) are audited, each either
  extended to enums or left as struct-only on purpose. `Type::struct_id()` keeps
  its meaning; add `Type::enum_id()`.
- `Optional<T>` is an `EnumKind::Optional(T)` made by the compiler on first use
  per `T` (generics do not exist before step 6; when they land it becomes the
  prelude's `enum Optional<T>` and `EnumKind::Optional` goes away). Its tags are
  fixed: `none` = 0, `Some` = 1. An all-zero `T?` is therefore `none`.
- No niche optimisation in step 5 (a `string?` is bigger than a `string`). Note
  it in `PLAN.md` "Performance" as a later item.

### 2.2 Enums in the MIR (package A.1 builds this; everyone else uses it)

- `Rvalue::Variant(EnumId, usize /*variant*/, Vec<Operand>)`: builds a value,
  like `Aggregate`.
- `Rvalue::Discriminant(Place)`: the tag as an unsigned integer of `tag_size`
  (for `Switch`/`If` tests).
- `Proj::Downcast(usize)`, always directly followed by `Proj::Field(i)`: the
  payload field `i` of that variant. **It appears only in operands that read a
  payload right after the tag test** (a `Copy` of a Copy field into a binding
  local), in `Drop` places made by the glue, and in the synthesised message
  functions (2.4). The builder never emits a `Move` out of a `Downcast`
  field in step 5 (owning payload bindings are E0901, Q3), with one exception
  made by the compiler itself: `maybe` on an optional or a failing call that is
  an rvalue (a temporary no name can reach) moves the variant's single field out
  and treats the temporary as moved (no other field exists to leak). Move paths stop at a
  `Downcast`: a move of the enum is a move of the whole enum. Validation rule
  V10 enforces "Downcast only under Copy/Drop/Ref-of-read, always followed by
  Field".
- `TerminatorKind::Switch` is emitted for `match` on an enum with three or more
  variants. Do **not** convert integer or text matches: they stay `If` chains
  (no risk to the existing goldens). Two-variant
  enums (`T?`, outcomes) use an `If` on `Discriminant(x) != 0` / `== 0`.
  Cranelift: `br_table` when the tags are dense, otherwise a compare chain; C:
  `switch`. Both backends implement `Switch` fully and `validate.rs` checks
  that every case value is a tag of the enum.
- Drop and copy glue: `drops::drop_glue_structs` / `copy_glue_structs`
  generalise to types: one glue function per owning enum, a switch on the tag
  that drops (or deep-copies) that variant's owning fields and the cause
  (2.4). A `Drop` of an enum place in a path where it may be uninitialised
  keeps working through the existing drop flags (an enum is one move path).
- Borrowck: a place with a `Downcast` overlaps every other place through the
  same enum local (conservative). Enum payloads that hold `&T`/`&mut T`/raw
  pointers are E0901 in step 5 ("a variant holding a borrow is not supported
  yet"), so `regions.rs` never sees region slots inside an enum.

### 2.3 The lowering of failures: a tagged outcome, no unwinding

A function written

```cpp
uint32 parse(string s) fails ParseError, Other { ... }
```

is compiled as if it returned an **outcome**: the compiler-made enum

```
Outcome#k { Ok(uint32 value) /* tag 0 */, Fail(F failure) /* tag 1 */ }
```

where `F` is the single failure type if the list has one entry, else the
compiler-made `FailSet#j { ParseError(ParseError e), Other(Other e) }` with one
variant per listed type (in the order written). Outcome and FailSet enums are
shared by `(ok type, fail type)` and by the list of failure types, so equal
signatures share layout and glue. A function that returns `none` has
`Ok` with no field. There is no hidden state, no thread-local, no unwinding:
a failure is an ordinary value moving up the call stack, and every drop on the
way runs in the normal return path.

How each piece lowers (typeck builds these HIR nodes, the MIR builder turns
them into the blocks below; the backends see only enums, `If` and `Switch`):

- **`return v;`** in a failing function: `_0 = Variant(Outcome, Ok, [v])`, then
  the ordinary return path (scope drops).
- **Falling off the end** of a failing `none` function: `_0 = Ok`.
- **`fail X;` / `fail X(args);` / `fail X because e;`**: build the failure value
  (`Variant(X, ..)`; with `because e` set its cause, 2.4), wrap it in the
  caller's `FailSet` variant if the set has more than one type, `_0 = Variant(
  Outcome, Fail, [that])`, then the ordinary return path.
- **A call to a failing function** is an ordinary `Callee::Func` call whose
  `dest` has the outcome type. Then:
  - `call maybe v` (and `fails with` chains, bare `fails`): `If Discriminant(out)
    != 0 { failure arm } else { result = Copy/Move (out as Ok).value }`.
    `maybe v` evaluates `v` only in the failure arm (lazy, SPEC). The failure
    value left in `out` is dropped at the end of the arm (its glue frees the
    strings and the cause chain).
  - bare `fails`: failure arm = convert the callee's failure to the caller's
    `F` (a move when both are the same type; a `Switch` re-wrapping variant by
    variant when the callee's `FailSet` is a subset of the caller's) and `return`
    it as `_0 = Variant(Outcome', Fail, [..])`. The conversion code is generated
    once per (callee set, caller set) pair as a small synthesised function.
  - `fails with (E e) => { block }` chains: failure arm = `Switch` on which
    failure type it is (a `FailSet` tag; for a single-type function there is no
    switch): the matching handler binds `e` (owned, moved out of the outcome: it
    is the whole failure value, so no partial move), runs its block; `e` is
    dropped at the end of the block unless moved by `because e`.
  - Inside a handler block `maybe v;` assigns the call expression's result local
    and jumps to the join block; `return` is the outer function's return;
    `fail ...;` is a failure of the outer function. A handler block that can fall
    off its end without any of these is E0547 unless the call returns `none`.
- **`main`**: the C `main` / Cranelift entry (`codegen_common::emit_c_main`,
  `cgen_common::main_fn`) become: call user `main`; if it is failing and the
  outcome tag is not 0, call `drachm_unhandled(message_string, cause, exit_code)`
  (runtime, noreturn) which writes the report to stderr and exits with the
  failure's exit code (default 1). A `main` that returns a number and fails:
  on success its value is the exit code; on failure the failure's exit code wins.
  Everything is dropped before the exit (the failure value is consumed by the
  report; leak check stays clean).
- **Unhandled calls** (`unhandled_fail = error`, the default): a call to a
  failing function (user function, method, constructor, `::to<T>()`) that is
  not directly followed by `maybe`, `fails` or `fails with` is E0544. Not a
  warning, not crash mode (`crash` is wired in step 7 with the config file).
  A failing call as an argument or operand needs its own suffix:
  `f(parse(a) maybe 0)`. The suffix attaches to the **call just before it**
  (Q15 for the extent of the fallback).

Every case is an ordinary MIR path; the move/borrow/drop analyses need no new
rule beyond enums. Checks that **must** still hold, with tests: a string alive
in the failing function is dropped on a `fail` path; a handler that `return`s
drops the call's temporaries and the other locals; nothing is dropped twice
when `maybe` takes the Ok arm.

### 2.4 Failure types, messages, causes, exit codes

```cpp
failure ToolError {
    NoInput              "no input file"          exit 2,
    BadConfig(string k)  "bad config key ${k}"    exit 3,
    LoadFailed           "couldn't load save",
}
```

- A `failure` is an enum (`EnumKind::Failure`) with extra per-variant data: its
  message (a template with `${name}` of that variant's values) and `exit`
  (default 1; range 1 to 255, E0552). It is a separate kind of declaration, so
  `enum` cannot follow `fails` (E0540). A failure value **can** be matched on
  like an enum (same machinery); that is how a handler will read a payload
  once Q3 and Q14 are decided.
- Message and exit code are compiled to two synthesised HIR functions per
  failure type, in the pattern of `auto_eq`/`eq_func` in `typeck.rs`:
  `string message(&F e)` (a `switch` on the tag; each arm is a `Join` of
  literal pieces and `Payload` places, using `HIR ExprKind::Payload`) and
  `uint8 exit_code(&F e)`. `${e}` in user code calls `message`. For a `FailSet`
  both dispatch to the member's function. (Q11: the template is a plain string
  that interpolates, against the rule "plain strings never interpolate".)
- `ExprKind::Payload { value: Box<Expr>, variant, field }` is a place: the
  payload field of a value known to be that variant. **Only synthesised code
  uses it**; it lowers to `Downcast`+`Field` operands (Copy numbers, or a
  borrowed `string` place for `Join`).
- **Causes.** Every failure value has a hidden cause word (`has_cause`): null,
  or a pointer to a heap chain node `{ drachm_str message; node *next }`. In
  the runtime (`runtime.c`):

  ```c
  void *drachm_cause_new(drachm_str *message /*moved in*/, void *next /*chain moved in*/);
  void  drachm_cause_drop(void *chain);                 // frees the whole chain
  _Noreturn void drachm_unhandled(const drachm_str *message, void *cause, int32_t code);
  ```

  `fail LoadFailed because e;` evaluates `message(&e)` into an owned string,
  takes `e`'s own cause chain, builds `drachm_cause_new(message, e.cause)` and
  stores it as the new failure's cause; `e` is **moved** (E0301 if used after).
  A cause is therefore text only, not inspectable (Q14). Drop glue of a failure
  calls `drachm_cause_drop`. Counting allocations: causes go through
  `drachm_str_alloc`/`drachm_str_drop` so `DRACHM_CHECK_LEAKS` covers them.
- **The report** (`drachm_unhandled`), interim format (Q12), to stderr:

  ```text
  couldn't load save
    because: no such file "save.dat"
      because: permission denied
  ```

  one line per link, each deeper link indented two more spaces, then exit with
  the code. No `error:` prefix, no colour.
- `${e}` and `e::text()`-style printing give the message only, never the
  `because` lines (Q12).
- Failures are never Copy (they own the cause word); they move.

### 2.5 Text to number becomes a failing call

`line::to<uint32>()` is typed as a call that fails with the built-in
`ConvertError` (interim name and shape, Q13), which lives in a prelude
(`src/prelude/core.drh`, loaded like `si.drh` by `typeck::check` before the
user's items, always in scope, no import needed; a user item with the same name
is E0202):

```cpp
failure ConvertError {
    BadText(string text, string kind)  "\"${text}\" is not a valid ${kind}",
}
```

So `n = line::to<uint32>();` alone is E0544, and `line::to<uint32>() maybe 0`,
`... fails`, `... fails with (ConvertError e) => { ... }` work. The runtime
parse functions (`drachm_str_parse_int/f64/f32`) are unchanged: the lowering
is `If parse_ok { Ok(v) } else { Fail(BadText(copy of text, "uint32")) }`. The
existing exit-101 panic path in `Callee::Parse` is deleted. All existing tests
and site examples that rely on the 101 stop (`tests/run/in_parse_bad.*`,
`site/examples/input-*.drh`, `reference-io.md`, `lesson-3-*`) are rewritten by
S5D in one commit.

### 2.6 Constructors, `fill` and `def` in the pipeline (summary; details in S5B)

- A constructor is a function of the struct's name; its body runs with `self`
  an incomplete local of the struct type (the same "incomplete value" state
  that `fill` blocks use, step 5B.2): field assignments give fields a value, and
  the function returns `self`. HIR: `Func.kind = Constructor { struct }`, returning
  the struct (or an outcome when it also has `fails`).
- Overloads are resolved at compile time by the ordering exact, lossless, lossy
  (W0102), the same code as operator overloads (`operator_call`), now shared in a
  new `src/typeck/overload.rs`.
- `fill`/`def` are checked in typeck (HIR: a `Stmt::Fill { local, partial, with:
  Option<DefId>, body }` that the MIR builder lowers to the plain field
  assignments, with the completeness check on the MIR's per-field "maybe
  initialised" analysis the borrow of `take` already uses).

## 3. Order, parallelism, file ownership

Two lanes, at most **two compiling agents at once**. The agents of one lane run
one after another. A package starts only when everything it needs has merged.

```
Lane 1 (strong model):   S5A.1 ──> S5C ──────────────> S5E (optional)
Lane 2 (standard model): S4R ──> S5B.1 ──> S5B.2 ──> S5A.2 ──> S5D
```

| Needs | Because |
|---|---|
| S5C needs S5A.1 | outcomes and failure types are enums; `Switch`, glue, `Variant` |
| S5A.2 needs S5A.1 | `T?` is an enum kind |
| S5B.2 needs S5B.1 | constructors share the incomplete-value state |
| S5D needs S5C, S5B.1 | handler chains extend `maybe`/`fails`; failing constructors |
| S5E needs S5C | it changes the outcome ABI after it works |
| S4R needs nothing | independent (runtime strings, control.rs) |

Expected shape of the schedule: lane 1's S5A.1 is the longest single package;
lane 2 does S4R, S5B.1, S5B.2 and S5A.2 meanwhile. If S5C is not merged when
lane 2 reaches S5D, lane 2 waits (do not start S5D on a branch of S5C).

File ownership. "Shared hunk" means both lanes edit the file; each edit is a
small disjoint hunk and the later merger rebases.

| File | S4R | S5A | S5B | S5C | S5D | S5E |
|---|---|---|---|---|---|---|
| `src/parser.rs` | | enum decl, `T?`, patterns | ctor members, `new..of`, `fill`/`def` | `failure`, `fails`/`fail`/`maybe` | `fails with`, `otherwise`, `because` | |
| `src/ast.rs` | | enum nodes | ctor/fill/def nodes | failure, fails nodes | handler nodes | |
| `src/typeck.rs` | | shared hunk (types) | shared hunk (`construct`, `init`) | shared hunk (calls) | shared hunk | |
| `src/typeck/enums.rs` (new) | | owns | | | | |
| `src/typeck/construct.rs`, `overload.rs`, `fill.rs` (new) | | | owns | | | |
| `src/typeck/failures.rs` (new) | | | | owns | owns (handlers part) | |
| `src/typeck/control.rs` | owns | `match` on enums: **shared hunk** (new fn) | | | | |
| `src/typeck/strings.rs` | owns (StrFn arms) | | | | `parse_text` | |
| `src/hir.rs` | StrFn | EnumDef, Variant, Payload | Fill, ctor kind | Fail, Try nodes | Handler nodes | |
| `src/types.rs` | | `Type::Enum` | | | | |
| `src/mir/{mod,build,validate,print}.rs` | tables | Variant, Discriminant, Downcast, Switch | Fill lowering | outcome lowering | handler lowering | call ABI marker |
| `src/mir/{moves,drops,mutability,borrowck}` | | glue, enum paths | incomplete-value analysis | | | |
| `src/codegen_mir.rs`, `src/cgen_mir.rs`, `*_common.rs` | StrFn arms | enum layout, Switch, glue | | `main` wrapper | | owns the call/return code |
| `runtime/runtime.c` | | | | cause API, report | | |
| `runtime/strings*` | owns | | | | | |
| `src/prelude/core.drh` (new) | | | | | owns | |
| `src/diag/codes.rs` | | E052x, E053x | E050x | E054x | E055x | |
| `tests/run.rs`, `tests/backends.rs` | | | | | | |
| `site/content/*` | control-flow | enums pages | constructors pages | errors pages | errors pages | |
| `docs/PLAN.md`, `docs/SPEC.md` | row 4 | row 5 note | row 5 note | row 5 note | row 5 note | |

`tests/run.rs` and `tests/backends.rs` need no edits for step 5 (cases are
dropped in as files). `src/diag/codes.rs`: each package adds its own contiguous
block of codes, with an `explain` text of the existing style (one short
paragraph, one example). `docs/SPEC.md` is changed by a package only to add the
status sentence in section 10's code-range list (`05xx`) and to fix a place
where the code differs from what SPEC says (report such differences; the owner
decides).

Every package: one commit per numbered step in its brief, `cargo test` green
before each commit, golden `.out` files read before blessing.

## 4. How to measure speed (S5C records the baseline, S5E improves it)

The failure lowering adds work on every call of a failing function: a larger
return value (written by the callee, tested and read by the caller) and a tag
test. The benchmark is `bench/fails.drh` with `fails.c` and `fails.rs` beside
it, added by **S5C** (the same layout as the other benchmarks; `bench/run.sh`
picks them up):

1. `leaf`: `uint64 step(uint64 x) fails Odd { if (x % 1000003 == 7) { fail Hit; }
   return x * 3 + 1; }` called 200 million times in a loop, results `maybe 0`
   (the failure path taken about once in a million). Measures the hot success
   path.
2. `chain`: three levels, each propagating with bare `fails`, the failure taken
   once per 10 000 calls. Measures propagation (the re-wrapping of a `FailSet`).
3. `strings`: a failing function that returns a `string`, so the outcome owns
   memory; ten million calls, 1% failures with a payload string.
4. `plain`: the same loops with a non-failing function, as the control.

The C version returns `struct { uint8_t tag; uint64_t v; ... }` by value and,
as a second variant, through an out pointer; the Rust version returns
`Result<u64, E>`. Report the `bench/run.sh` table (drachm -O, C, Rust, plain
Cranelift) **before** (S5C, with the plain lowering) and **after** (S5E, if
done). Targets: for `leaf`, drachm -O within 1.3x of the C struct-by-value
version; plain build no more than 3x slower than its own `plain` control.
These are targets to report against, not gates; the gate is that both backends
behave identically. Also report compile time of `cargo test` before and after
each package (`time cargo test`, once).

## 5. Site rules for the whole step

The design pages `design-constructors.md`, `design-enums.md`, `design-errors.md`
carry `status: not-implemented` at the top and per-section markers
(`<!-- status: tentative -->`). When a package makes a section build:

1. Move that section into the matching **reference** page (new
   `reference-enums.md`, `reference-errors.md`; extend `reference-structs.md`
   with "Constructors" and "Filling and defaults"; `reference-types.md` with
   `T?`; `reference-control-flow.md` with `match` on enums), with a real
   example in `site/examples/NAME.drh` + `.out`, and the real compiler output
   for its errors (the format already used in `reference-ownership.md`).
2. In the design page, delete the moved section and leave a one-line link, or
   replace the page-level `status:` front matter by a `<!-- status: not-implemented -->`
   marker under each heading that still is not built. The page-level status
   goes only when nothing on the page is unbuilt.
3. Update the page's lead paragraph ("the compiler does not accept any of it
   yet") to be true.
4. Roadmap (`site/content/coming-later.md`): the progress table's row 5, and
   the "coming later" bullets. Add the new reference pages to
   `site/content/order.json`.
5. Lessons: S5A adds `lesson-10-enums.md` (enums, `match` on them, optionals;
   exercises with `solution-10-N`); S5C+S5D add `lesson-11-failures.md`.
   Pattern: `lesson-8-ownership.md`.
6. `site/gen-errors.sh` after every change to `src/diag/codes.rs`.
7. `reference-diagnostics.md` lists the new codes.
8. Every new example compiles silently, is leak-free, and prints exactly its
   `.out` on both builds (`tests/run.rs` checks it).

## 6. New diagnostic codes

A new range is introduced: `05xx` = constructors, enums, optionals, failures
(add the line to the header comment of `src/diag/codes.rs`, to SPEC section 10's
range list, and to `reference-diagnostics.md`). Wording style is SPEC section
10 / E0302: a message that states the problem with the user's names, a primary
label that says what is wrong **here**, secondary labels that show the other
place involved, and a `help:` with code the user can type. Every message is
written for the owner's batch review (the same status as the 03xx wording).
Names are the user's spelling; types print as the compiler prints them.

Existing codes reused: E0101 (type mismatch), E0103 (arguments), E0114 (values
for a shorthand), E0201 (no such field), E0202 (duplicate declaration), E0210
(contains itself; reword its message from "struct" to "`Tree`" so it serves
enums), E0301 (used after move), E0332 (part of a value that has none; its
`help` changes to point at `fill`), W0102, W0110/W0111 (now also for enums, with
the missing variants listed), E0901.

| Code | Package | Situation | Message | Primary label | Help |
|---|---|---|---|---|---|
| E0501 | B.1 | no overload fits | no constructor of `Circle` accepts these values | given `string` (one label per value that does not fit) | `Circle` takes `(float64)` or `(uint32)`; secondary labels at each constructor: "this constructor takes (float64)" |
| E0502 | B.1 | two fit equally | more than one constructor of `Port` fits `5` equally well | both can take `uint8` without losing anything; secondary labels on the two constructors | convert the value yourself: `5::to<uint16>()`, or name the type: `5u16` |
| E0503 | B.1 | same signature twice | `Circle` has two constructors taking `(float64)` | defined again here; secondary: first defined here | change the parameter types, or remove one |
| E0504 | B.1 | field not set | the constructor of `Player` ends without giving `hp` a value | `hp` has no value when the constructor ends (at the `}`, or at the `return`); secondary: `hp` is declared here | add `self.hp = ...;` on every path that reaches the end |
| E0505 | B.2 | `fill` incomplete | `fill` ends without giving `hp` a value | `hp` has no value at this `}`; secondary: `hp` is declared here | give it a value in the block, or write `fill partial p { }` and fill the rest later |
| E0506 | B.2 | `fill` on a value | `p` already has a value, so it cannot be filled | `fill` starts a value from nothing; secondary: `p` is given a value here | declare it without a value (`Player p;`), or change a field of a `mut` binding (`p.hp = 5;`) |
| E0507 | B.1/B.2 | use of incomplete value | cannot use `p` before it is complete | used here; secondary: `name` still has no value | give the remaining fields a value in a `fill` block first; in a constructor read the parameter instead of `self.r` |
| E0508 | B.2 | unknown def | there is no `def` named `Veteran` | no such def; | the defs of `Player` are `PlayerStart`, `Veteran` (or: `Player` has no defs) |
| E0509 | B.2 | def declared wrong | `def PlayerStart` gives `hp` twice (or: there are two defs named `PlayerStart`) | given again here; secondary: first given here | remove one |
| E0510 | B.2 | no default | `Player` has no default `def` | `fill ... with defaults` needs one | write `struct Player defaults PlayerStart { }`, or `def Player { }` with `struct default Player { }` |
| E0520 | A.1 | duplicate variant | `Shape` has two variants named `Rect` | defined again here; secondary: first defined here | rename one |
| E0521 | A.1 | unknown variant | `Shape` has no variant `Triangle` | no such variant | did you mean `Rect`? / its variants are `Circle`, `Rect`, `Empty` |
| E0522 | A.1 | payload count | `Rect` takes 2 values, but 1 is given | needs `float64 w, float64 h`; secondary: `Rect` is declared here | write `Rect(2.0, 3.0)` |
| E0523 | A.1 | bare variant, no type | cannot tell which enum `Circle` belongs to | `Circle` needs a type to come from | write `Shape::Circle(1.0)`, or give the type: `Shape s = Circle(1.0);` |
| E0524 | A.1 | name clash | the variant `Circle` has the same name as the struct `Circle` | variant defined here; secondary: the struct | rename one of them: a bare `Circle(..)` could mean either |
| E0525 | A.1 | wrong pattern | `Wall` is not a variant of `Shape` | this `case` cannot match a `Shape` | the variants of `Shape` are `Circle`, `Rect`, `Empty` |
| E0526 | A.1 | binding type | `Rect` holds `float64` here, not `string` | this binding is `string` | write `float64 w` |
| E0530 | A.2 | `none` where none fits | `none` needs an optional type, but `uint32` is expected | this is `none` | make the type optional: `uint32?` |
| E0531 | A.2 | optional as plain | `age` may be none, so it cannot be used as a `uint32` | this is `uint32?` | give a fallback: `age maybe 0`, or look inside it with `match (age)` |
| E0532 | A.2 | `maybe` of nothing | `maybe` needs an optional or a call that can fail, but this is a `uint32` | this value cannot be missing | remove `maybe` |
| E0540 | C | not a failure | `fails` names `Shape`, which is not a failure type | not a failure type (or: no such failure) | declare one: `failure Shape { ... }` |
| E0541 | C | `fail` unlisted | `parse` does not list `Empty` after `fails` | this failure is not in the list; secondary: the signature | add it: `fails ParseError, Empty` |
| E0542 | C | `fail` without `fails` | `check` can fail here, but it does not say `fails` | failure raised here; secondary: `check` is declared here | add `fails ParseError` after the parameters |
| E0543 | C | ambiguous `fail` | `Empty` is a variant of more than one failure this function lists | ambiguous | write `fail ParseError::Empty;` or `fail LoadError::Empty;` |
| E0544 | C | unhandled call | `parse` can fail, and nothing says what to do when it does | this call can fail with `ParseError` | fall back: `parse(s) maybe 0`; pass it up: `parse(s) fails` (and `main` says `fails ParseError`); or handle it: `parse(s) fails with (ParseError e) => { ... }` |
| E0545 | D | handler for the impossible | `parse` cannot fail with `FileMissing` | nothing to handle; secondary: `parse` lists `ParseError` | remove this handler |
| E0546 | D | handled twice | `ParseError` is handled twice | handled again here; secondary: first handled here | merge the two handlers |
| E0547 | D | no value, no exit | this handler neither gives a value nor leaves | the block can end here without a value | end it with `maybe 0;`, or leave with `return` or `fail ...;` |
| E0548 | C | `fails` not declared | `main` does not declare `ParseError`, so `fails` cannot pass it up | the failure is passed up here; secondary: `main` is declared here | add `fails ParseError` to the signature, or handle it |
| E0549 | D | `because` of a non-failure | `because` needs a failure, but `n` is a `uint32` | not a failure | `because` takes the failure you caught: `because e` |
| E0550 | D | `maybe v;` alone | `maybe` with a value as a statement only works inside a `fails with` block | there is no handler to give a value to | to fall back, write `call() maybe value` |
| E0551 | C | message names | the message of `BadConfig` uses `${port}`, but it has no such value | no value `port`; secondary: the variant | its values are `k` |
| E0552 | C | exit range | exit code `300` is out of range | exit codes are 1 to 255 | write a code from 1 to 255 |
| E0553 | D | `otherwise` position | `otherwise` must come last, and only once | misplaced; secondary: the other | move it to the end |
| E0554 | C | list problem | `parse` lists `ParseError` twice (or: `failure Empty` has no variants) | listed again here; secondary: first listed here | remove one |

Full renderings to copy the layout from (the implementers write the real
output; these fix the shape):

```text
error[E0544]: `parse` can fail, and nothing says what to do when it does
  --> main.drh:7:20
   |
 7 |     uint32 n = parse(line);
   |                ^^^^^^^^^^^ this call can fail with `ParseError`
   |
   help: give a fallback with `parse(line) maybe 0`, pass the failure up with `parse(line) fails`
         (and add `fails ParseError` to `main`), or handle it with
         `parse(line) fails with (ParseError e) => { ... }`
```

```text
error[E0501]: no constructor of `Circle` accepts these values
  --> main.drh:9:16
   |
 3 |     Circle(float64 radius) { self.r = radius; }
   |     ------------------------------------------ this constructor takes (float64)
 4 |     Circle(uint32 radius)  { self.r = radius; }
   |     ------------------------------------------ this constructor takes (uint32)
   |
 9 |     Circle a = "five";
   |                ^^^^^^ this is `string`
   |
   help: `Circle` takes (float64) or (uint32)
```

```text
error[E0505]: `fill` ends without giving `hp` a value
  --> main.drh:6:5
   |
 2 |     uint32 hp;
   |            -- `hp` is declared here
 5 |     fill p { p.name = "Sam"; }
   |                              ^ `hp` has no value at this `}`
   |
   help: give it a value in the block, or write `fill partial p { }` and fill the rest later
```

```text
error[E0521]: `Shape` has no variant `Triangle`
  --> main.drh:8:25
   |
 8 |     Shape s = Shape::Triangle(1.0);
   |                      ^^^^^^^^ no such variant
   |
   help: its variants are `Circle`, `Rect` and `Empty`
```

The warning W0110 for a `match` on an enum that leaves variants out:

```text
warning[W0110]: this `match` does not handle `Rect` and `Empty`
  --> main.drh:12:5
   |
12 |     match (s) {
   |     ^^^^^ these variants are missing
   |
   help: add `case Rect(float64 w, float64 h) { }`, `case Empty { }`, or `case _ { }`
```

## 7. Reporting

Each package's report lists: what differs from the brief, the benchmark table
where the brief asks for one, `time cargo test` before/after, every interim
that the code took (with its file and line), every new language question found,
and the `grep` results the "Done when" asks for.

## 8. Language questions (the owner decides; answers go to `owner-answers.md`)

Every question has its interim, the behaviour the code has until answered. The
interim never silently picks a design: it is an error saying "not supported
yet" (E0901) or the more restrictive reading. The option marked "(original)" is
a Drachm idea rather than a borrowed one; none of them is marked as
recommended.

### Q1. Matching a literal `*` in a text pattern (step 4)

```cpp
match (name) {
    case "*.png"  { }     // wildcard star
    case "a*b"    { }     // a literal star: how?
}
```

- A. Backslash escape in the pattern: `case "a\*b"` (needs `\*` as a legal
  escape in every string, E0008 today).
- B. Doubling: `case "a**b"` is a literal star, a single `*` a wildcard.
- C. (original) A separate pattern word: `case literal "a*b"` matches the text
  exactly, no wildcards; `case "a*b"` is always a glob.

Interim: every `*` is a wildcard; a literal star cannot be matched.

### Q2. What `c` is in `for (string c of s)`

```cpp
for (string c of text) { io::outln(c); }
string first;                      // can the loop variable be kept after the loop?
```

- A. An ordinary owned `string` (a copy per character; one-byte ASCII
  characters cost nothing). It can be kept, moved, returned.
- B. A borrowed view: `for (&string c of text)`; the `string` form is an error
  ("the characters are borrowed, write `&string c`"). No copies, but nothing
  can be kept without `copy c`.
- C. (original) Both, chosen by the type written: `for (string c of text)` gives each
  character as an owned copy you may keep, `for (&string c of text)` is the
  borrowed view; the compiler picks the cheap path when the body never keeps `c`.

Interim: A.

### Q3. Binding a payload that owns memory in an enum `case`

```cpp
enum Msg { Text(string body), Quit }
match (m) {
    case Text(string body) { io::outln(body); }   // what does this do to m?
    case Quit              { }
}
```

- A. The binding moves the payload out and `m` is used up (like `take`); after
  the `match`, `m` cannot be used.
- B. Bindings borrow: the pattern says so, `case Text(&string body)`; `m` stays whole.
- C. (original) The pattern says what it does with the word `take` or `copy`:
  `case Text(take string body)` moves, `case Text(copy string body)` copies,
  and a bare `string body` is an error with both fix-its (it matches how `take`/`copy` already work everywhere else).

Interim: bindings of `string`-owning payloads, and structs that own strings,
are E0901 ("binding a payload that owns memory is not supported yet"); numbers,
bools and Copy structs bind by value; `case Text(_)` and `case Text` (ignore the
payload) work.

### Q4. Patterns beyond one level

```cpp
match (s) {
    case Circle(float64 r) if r > 10.0 { }       // guard
    case Rect(float64 w, float64 w2), Square(float64 w) { }   // one body, two variants
    case Some(Circle(float64 r)) { }             // nested
    case Rect(2.0, float64 h) { }                // a value inside a pattern
}
```

- A. None of them: one pattern per variant, bindings only; use `if` inside.
- B. Guards with `if` after the pattern; nothing else.
- C. (original) Value tests go in the binding list with a `=`-free form: `case Rect(w is 2.0, float64 h)`, no guards, no nesting.

Interim: A; the other forms are E0901.

### Q5. `T` to `T?`, and testing for none

```cpp
uint32? find(string key) { return 5; }       // wraps automatically?
uint32? x = 5;
if (x == none) { }                            // allowed?
```

- A. `T` converts to `T?` automatically everywhere a `T?` is expected (return,
  declaration, assignment, argument); `x == none` and `x != none` allowed.
- B. Wrapping is written: `Some(5)`; testing is done with `match`.
- C. (original) Wrapping is automatic only when the written type is `T?`, and
  the test is a word: `if (x has value)`, `if (x is empty)`.

Interim: B (`Some(5)` works as a bare variant from the expected type); an
automatic wrap or `== none` on an optional is E0901 with the fix-it.

### Q6. `==` on enums, and methods on enums

```cpp
Shape a = Empty;
if (a == Empty) { }                           // allowed?
enum Shape { Circle(float64 r), Empty,  float64 area() { ... } }   // methods in the enum body?
```

- A. `==` is automatic (same variant, equal payloads), like structs; enums have
  methods like structs.
- B. `==` only for enums without payload; methods are plain functions taking the enum.
- C. (original) Neither: enums are matched, never compared, and behaviour lives
  in a `struct` that holds the enum.

Interim: `==` on an enum is E0901 (use `match`); methods in an enum body are a
syntax error with help "write a function that takes the enum".

### Q7. Variant names and the places they can be written bare

```cpp
struct Circle { float64 r; }
enum Shape { Circle(float64 r), Empty }       // same name as the struct
Shape s = Circle(1.0);                        // which Circle?
io::outln(area(Empty));                       // bare variant as an argument: type known from area's parameter
uint32 n = area(Circle(1.0))::to<uint32>();   // the parameter type decides here
```

- A. Clashes are an error at the declaration; a bare variant needs an expected
  type from the declaration, assignment, argument or return.
- B. The expected type decides first; if there is none, the struct/function wins
  and a bare variant is an error with `Shape::Circle`.
- C. (original) Bare variants always need a leading marker: `.Circle(1.0)` (a
  dot, resolved from the expected type), plain `Circle(1.0)` always means the
  struct or function.

Interim: A (E0524, E0523).

### Q8. Constructors next to the automatic one

```cpp
struct Person { string name; uint32 age;
    Person(string name) { self.name = name; self.age = 0; } }
Person a = "Sam";              // the written one
Person b = "Sam", 30;          // is the automatic field-order constructor still there?
Person c = name: "Sam", age: 3;    // named values with a written constructor?
Person(string n) { ... Person(n, 0) ... }   // one constructor calling another?
```

- A. Written constructors replace the automatic one; names pick a constructor by
  parameter names; no delegation.
- B. The automatic constructor always stays; names go with fields; delegation
  by calling `self = new Person of (n, 0);`.
- C. (original) The automatic one is asked for by name: `Person(auto);` inside
  the struct brings it back; named values match parameter names of the chosen
  overload.

Interim: A (the automatic constructor exists only when no constructor is
written, E0501 explains); named values only with the automatic constructor
(E0901 with a written one); delegation is E0901.

### Q9. May a constructor or `fill` read what it has already set

```cpp
Circle(float64 radius) { self.r = radius; self.area = self.r * self.r * 3.14; }  // reads self.r
fill p { p.hp = 5; p.max = p.hp * 2; }
```

- A. Yes, a field that already has a value can be read, method calls on the
  incomplete value are not allowed.
- B. No: until the value is complete only assigning is allowed; use the
  parameter or a local (the owner's "no reads" sentence, taken literally).
- C. (original) Reads allowed through a word: `fill p { p.hp = 5; p.max = now p.hp * 2; }`.

Interim: B (E0507).

### Q10. `fill ... with <name>` and what a `def` may contain

```cpp
def Veteran of Player { hp = 200; title = "vet"; }
fill p with Veteran { p.name = "Sam"; }       // owner's reading, not confirmed
def PlayerStart of Player { hp = 100; created = time::now(); }   // calls? other fields?
```

- A. Named defs can be used (`with Veteran`); a def's values are any
  expressions that do not read fields.
- B. Only `with defaults`; a def holds constants and string literals.
- C. (original) A def may extend another: `def Veteran of Player from PlayerStart { hp = 200; }`.

Interim: `with defaults` only (the rest E0901); def values may be any
expression that does not mention fields, evaluated each time it is used.

### Q11. Failure messages interpolate in plain strings

SPEC section 7 writes `"bad config key ${k}"` as a plain string; SPEC section 4
says plain strings never interpolate.

- A. Exception: a failure message is a template and always interpolates.
- B. Failure messages are written as `f"bad config key ${k}"` like everywhere
  else (the SPEC example changes).
- C. (original) The values are named with `%` as in `io::outf`: `"bad config key %k"`.

Interim: A (the message position is parsed as an interpolating string; the
site shows the SPEC form).

### Q12. How an escaping failure is reported, and what `${e}` prints

```cpp
none main() fails ToolError { ... fail LoadFailed because e; }
```

```text
couldn't load save
  because: no such file "save.dat"
```

- A. stderr, the message, then one `because:` line per cause indented two more
  each, nothing else; `${e}` prints the message only.
- B. As A with a prefix (`error: couldn't load save`) and `${e}` prints the
  whole chain.
- C. (original) The program chooses: `e::report()` returns the whole chain as
  text (message plus `because:` lines) for the program to print or log; `${e}`
  is the message only; an escape from `main` prints the report.

Interim: A.

### Q13. The failure of `::to<T>()`, and its name

```cpp
uint32 n = line::to<uint32>() fails with (ConvertError e) => { ... };
```

- A. A built-in `ConvertError` with one variant `BadText(string text, string kind)`.
- B. One variant per cause: `Empty`, `NotANumber`, `OutOfRange`, so a program can tell them apart.
- C. (original) No shared type: `to<T>()` fails with a failure type named after
  the target, `ConvertError<uint32>` is not possible before generics, so each
  number type gets its own (`Uint32Error`, `Float64Error`), which handlers can
  tell apart.

Interim: A.

### Q14. How much a handler can do with the failure

```cpp
parse(s) fails with (ToolError e) => { ... }          // read BadConfig's k? the cause?
parse(s) fails with (BadConfig e) => { ... }          // handle one variant only?
parse(s) fails with (any e) => { ... }                // what can `any e` do?
```

- A. By type only. Payload through `match (e)`. `any e` supports `${e}`,
  `because e` and nothing else. Causes are text only.
- B. Handlers may name a variant (`(BadConfig e)`); `any e` can also be re-raised
  with `fail e;` when the caller lists it.
- C. (original) `e` has fixed members: `e::message()`, `e::exit()`, `e::cause()`
  (the cause as another failure value).

Interim: A (the payload of an owning variant also needs Q3).

### Q15. How far a `maybe` fallback reaches

```cpp
uint32 a = parse(s) maybe 0 + 1;         // (parse maybe 0) + 1, or parse maybe (0 + 1)?
uint32 b = parse(s) maybe n * 2 + k;
```

- A. A parenthesised or single-operand fallback only; anything else after it
  needs parentheses (error says so).
- B. The fallback is the rest of the expression (lowest precedence).
- C. (original) The fallback ends at the next operator unless in `( )`, and the
  whole `call maybe v` is itself an operand.

Interim: A (E0003 with a parentheses fix-it).

### Q16. Ignoring a failure of a call that returns `none`

```cpp
fs::close(f) maybe;            // swallow it?
fs::close(f) maybe none;
```

- A. `maybe` with nothing after it means "ignore the failure".
- B. Not allowed; write a handler with an empty block: `fails with (any e) => { }`.
- C. (original) A word: `call() ignore failure;`.

Interim: B (and `maybe` alone is E0003).

### Q17. Where a failing function may appear

```cpp
bool ==(Port o) fails X { ... }       // failing operator
uint32 n = (parse(s) maybe 0) + 1;    // inside operators: fine
```

- A. Anywhere a call is allowed, operator methods included.
- B. Not operator methods, `drop()` or `key()`.

Interim: B (E0901 for operators; `drop()` stays E0328).

### Q18. Exit codes of failures

```cpp
LoadFailed "couldn't load save" exit 0,     // exit 0 for a failure?
none main() fails ToolError { }             // main also returns a number?
```

- A. 1 to 255 only (E0552); the failure's code wins over a number returned by `main`.
- B. 0 to 255; same winning rule.
- C. (original) The exit code is part of the type: a failure with no `exit` uses 1, and `main` may not both return a number and fail.

Interim: A.
