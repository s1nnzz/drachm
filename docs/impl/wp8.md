# WP8: `lives` on functions and structs, signature inference

Implementer: **needs a strong model for the integration (steps 3 and 4)**, a
standard one is enough for steps 1 and 2. The algorithm is small, but it sits
on WP7's solver and its errors are explanations that cross functions.

Read first: `step3-ownership.md` 3.6 (all), 2.4, 3.5.3 C5 and C6, WP8 in 6,
section 4 (E0307 to E0309, E0314, E0320); owner answers Q9, Q10, Q10ii, Q18 in
`owner-answers.md`; `wp7.md` (the solver and the `analyse_body` interface you
call); and the code: `src/typeck.rs` (`check_struct_lives`, the function `lives`
checks near `unsafe_disabled`), `src/mir/mod.rs` (`LivesSig`), `src/hir.rs`
(`Func.lives`, `StructDef.lives`). This brief wins over the spec.

## Goal

1. A function that returns a borrow and does not write `lives` gets its
   signature from its body (Q9): one parameter it can borrow from, inferred;
   several, E0308 with the fix-it. A written `lives` is checked against the body
   (E0309). A borrow of one parameter stored into another is E0320.
2. Callers use the signature (C5): the result keeps the listed arguments'
   loans alive, so `r = longest(&x, &y)` with `y` dying early is E0306.
3. Recursion and mutual recursion work (fixed point).
4. Struct `lives` (Q10: field names, several allowed, each field may borrow
   from a different place) works end to end: most of the front end exists
   (`check_struct_lives`, E0314); this package adds the tests and the
   checker side (slots per `lives` field, already in WP7's `regions.rs`).
5. `LivesSig::Unknown` and WP7's `sigs::provisional` are gone.

## Parallelism with WP7 and WP9 (read this first)

WP8 depends on WP7 only through the interface in `wp7.md` step 3
(`borrowck::analyse_body`, `Solution`, constraints with origins, `LivesSig`
and `regions.rs`). Start as soon as WP7 step 2 is merged (the `LivesSig` enum
and `regions.rs`); you can write everything in steps 1 and 2 against a stub
`analyse_body` before WP7's solver exists, and finish step 3 when WP7 step 3 has
merged. Your run tests (`tests/run`) need WP7 step 6 (the gate lift); until then
write them as `tests/fail` cases and `build_unchecked` cases in `tests/backends.rs`,
and move them when the gate is lifted. Do **not** wait for WP9: nothing here
touches pointers.

File ownership (to avoid conflicts):

| Yours alone | Shared, edit only what is listed |
|---|---|
| `src/mir/lives.rs` (new) | `src/mir/borrowck/sigs.rs`: you delete it (WP7 owns it until you do; do it in your last commit) |
| `tests/fail/own_*lives*.drh`, `tests/run/own_*lives*.drh` and the other tests below | `src/mir/check.rs`: replace WP7's one `borrowck::check_program` line by `lives::check_program` (a one-line hunk) |
| `site/content/reference-lives.md` (new) | `src/mir/build.rs` line ~61: `lives: ... LivesSig::Unknown` becomes `NoBorrowResult` or `Unknown` placeholder to be overwritten by `lives::check_program` (one hunk) |
| E0308, E0320 in `src/diag/codes.rs` (your own block) | `src/typeck.rs`: only the `lives` name checks (the function that reports E0309 for a name that is not a borrow parameter) for the `self` rule below |
| | `site/content/coming-later.md`, `order.json`, `reference-diagnostics.md`, `errors.md` (one bullet / one entry each) |

Never edit `borrowck/{constraints,solve,loans,conflicts,explain}.rs`. If you need
something from the solver, ask WP7's implementer to add it to `Solution`.

## What to build

One commit per step, `cargo test` green after each.

### Step 1. The call graph and the driver

`mir/lives.rs`:
```rust
pub fn check_program(hir: &hir::Program, mir: &mut Program, diags: &mut Vec<Diagnostic>);
```
- Build the call graph from `TerminatorKind::Call { callee: Callee::Func(f), .. }`
  of every body; Tarjan's SCCs; process in post-order (callees first); inside an
  SCC iterate in `FuncId` order.
- For each function the signature starts as: no borrow slot in the return type:
  `NoBorrowResult`; a written clause: `Declared(v)`; otherwise `Inferred([])`.
- Recursion (3.6.3): for an SCC with any function that needs inference, repeat
  { `analyse_body(report: false)` for each member with the current signatures; set
  each member's `Inferred` set to its `required`; } until no signature changes
  (sets only grow), then one final `analyse_body(report: true)` round whose
  diagnostics are kept.
- Unit-test the SCC order and the fixed point on hand-written call graphs.

### Step 2. Required sets and the three checks

For a body's `Solution`, 3.6.2 (the universal region check):
```text
required = {}
for each parameter universal U:
    for each end(V) in values[U].ends where V != U:
        if V is a return region: required += param_of(U)
        elif V is another parameter's slot and the relation is not implied by C6: E0320
```
Then per 3.6.3:
- **Declared:** `required ⊆ declared` else E0309 (wording below); extra declared
  names that nothing requires are accepted silently (they only keep more
  loans alive at call sites).
- **Not declared:** `|required| == 0` and the result has slots: record
  `Inferred([])` (the loan check has already reported E0307); `== 1`:
  `Inferred(required)`; `>= 2`: E0308, still record `Inferred(required)`.
- A function with errors in its body: record `Unknown` so callers assume
  nothing and report nothing.

Methods: parameter 0 is `self`. A getter `&string name() { return &self.name; }`
requires `{self}` and is inferred. A method whose result could come from `self`
**or** another borrowed parameter has `required = {self, key}`: this needs the
name `self` in `lives`, which is **not designed yet** (language question 2): the
code reports **E0901** "naming `self` in `lives` is not supported yet" instead of
E0308 when `self` is in `required`, and the same E0901 when a written `lives`
names `self` (today typeck reports E0309 for it; change that one message).

Wording, E0302 layout, written for the owner's review:

```text
error[E0308]: `longest` returns a borrow, but which parameter it borrows from depends on the path
  --> main.drh:1:10
   |
 1 | &string longest(&string a, &string b) {
   |         ^^^^^^^ the result borrows from `a` or `b`
 2 |     if (a::len() > b::len()) { return a; }
   |                                ------ returns a borrow of `a` here
 3 |     return b;
   |     ------ returns a borrow of `b` here
   |
 help: write `lives a, b` after the parameters: `&string longest(&string a, &string b) lives a, b {`
```
```text
error[E0309]: `pick` returns a borrow of `b`, but its `lives` names only `a`
  --> main.drh:1:49
   |
 1 | &string pick(&string a, &string b) lives a { return b; }
   |                                      ----- `lives` names only `a` here
   |                                                     ^^^^^^^^ returns a borrow of `b` here
   |
 help: add `b` to the list: `lives a, b`, or return a borrow of `a`
```
```text
error[E0320]: cannot store a borrow of `b` inside `a`
  --> main.drh:2:5
   |
 1 | none keep(&mut &string a, &string b) {
   |           ---------------  `a` can only hold borrows that last as long as the caller's value
 2 |     a = b;
   |     ^^^^^ the borrow of `b` is stored in `a` here
   |
 help: the caller cannot promise that `b` lives as long as what `a` points to; store a copy: `copy b`
```
(Check how `a = b` for `&mut &string a` actually lowers before finalising the
example; use whichever small program really produces the constraint, and keep the
wording.) Explanations walk the constraint graph backwards from the return
region to the parameter universal (3.6.5) using the constraint origins: the
`Return` origin is the "returns a borrow of ... here" span, the `Borrow` origin
the borrow span. When the path crosses a call, label the call: "through the call
to `first` here".

Add E0308 and E0320 to `codes.rs` with explain texts in the existing style.

### Step 3. Callers

C5 is WP7's, but you make it real: `check_program` writes the final signatures
into `mir.lives`; `analyse_body` reads `sigs`. Delete `sigs::provisional` and the
`Unknown` placeholder in `build.rs`. Run the whole suite: every program in
`tests/run`, `examples`, `site/examples` must stay free of borrow diagnostics
(WP7's false-positive net now uses real signatures).

### Step 4. Struct `lives`, tests and site

The front end is done; make sure all of Q10 is covered by tests (below). If a
run test exposes a slot-mapping bug (a struct with two `lives` fields, or a
struct inside a `&` borrow: `&Parser` has two slots), fix it in
`borrowck/regions.rs` **by asking WP7's owner**, or if WP7 has finished, by
editing it yourself with a small commit that says so.

## Tests

Fail (`tests/fail/*.drh` + `.err`): `own_missing_lives`, `own_wrong_lives`,
`own_lives_call_site` (from the spec WP8 list, verbatim), and
- `own_lives_store_param` (E0320)
- `own_lives_method_two` (a method whose result could come from `self` or a
  parameter: E0901 about `self`)
- `own_lives_struct_result` (`Parser start(&string a, &string b)` returning a
  struct holding one of them, no clause: E0308)
- `own_lives_declared_too_many` is accepted (`lives a, b` where only `a` is
  returned): a run test that proves callers keep both alive (so changing `b`
  while the result is live is an E0302 in the twin fail test
  `own_lives_conservative`)

Run: `own_lives`, `own_recursive_lives` (spec, verbatim), and
- `own_lives_mutual` (two functions calling each other, result from one parameter
  through the cycle)
- `own_lives_pair` (`struct Pair lives a, b { &string a; &string b; }` built from
  `&x` and `&y` where `y` goes out of scope before the pair's last use of `a` but
  after... i.e. the pair is dead before `y` dies: accepted; its fail twin keeps the
  pair alive past `y`: E0306 "kept in `p.b`")
- `own_lives_method_getter` (inferred through `self`)
- `own_lives_no_param_ok` (a function whose body returns a borrow of nothing
  borrowed and returns no borrow: no clause needed)

A **known limitation test** (a run test with a comment): `string r = rest(&p);`
where `rest(&Parser p)` returns `p.src`, followed by changing `p.pos` while `r`
is live is **rejected** by this design (a `lives p` keeps the whole of `p`
borrowed, language question 1). Keep it as `tests/fail/own_lives_whole_param.drh`
so the day the owner decides otherwise, the test flips.

## Site updates (CLAUDE.md; only what the compiler accepts)

- `site/content/reference-lives.md` (new, in `order.json` after
  `reference-ownership`): when `lives` is needed (only when ambiguous), reading
  the E0308 fix-it, `lives` on structs with several fields, E0308, E0309, E0320
  with real compiler output, and the conservative limit. Examples
  `site/examples/lives.drh`, `lives-struct.drh` + `.out`.
- `site/content/coming-later.md`: remove the lifetimes/`lives` bullet, leave
  `open` fields and `unsafe`.
- `site/content/reference-diagnostics.md`, `errors.md`: new codes, rerun
  `site/gen-errors.sh`.
- Link from `lesson-9-borrowing.md` (WP7) only if it exists when you merge.

## Done when

`cargo test` green; `grep -rn "Unknown\b.*Lives\|provisional" src` is empty;
the spec's WP8 acceptance programs behave as listed; the report lists the
limitation test and the two language questions.

## Pitfalls

- Method parameters are 0-based with `self` first; `hir.funcs[f].lives` indexes
  must be the same numbering as `LivesSig` (check once, write a test).
- A function that takes no borrow parameter but returns a borrow: required is
  empty and E0307 is WP7's.
- Fixed point over an SCC: errors must not be reported in the iterating rounds,
  or they print several times.
- Do not infer more than the spec: `lives` is about parameters, never fields or
  paths (Q10ii).
- Inferred sets are sorted by parameter index so diagnostics are stable.

## Language questions found

1. **What does `lives p` mean for a parameter that is itself a holder?** A
   function `&string rest(&Parser p)` returning `p.src` only needs what `p`
   *holds*, but a `lives p` that names the whole parameter (Q10ii, what is
   implemented) keeps `p` itself borrowed while the result lives, so `p.pos` cannot be
   changed until the result is dead. Until decided the code is the whole-parameter
   rule above (not E0901, because it is merely conservative).
   - A. Whole parameter (today). `lives p`.
   - B. Inner borrows only by default: `lives p` means what `p` holds; keeping the holder itself
     borrowed needs a second word.
   - C. Original: allow paths after all, `lives p.src`, only when the inner is wanted.
   ```cpp
   &string rest(&Parser p) lives p.src { return p.src; }   // C
   Parser p = &text, 0;
   string r = rest(&p);
   p.pos = 3;          // fine under B/C, error under A
   io::outln(r);
   ```
2. **How is `self` named in `lives`?** A method like `&string get(&string key)`
   on a struct may return either. Q9 says ambiguous means the programmer writes
   `lives`, but `self` has no spelling there. E0901 until decided.
   - A. `self` as a name: `&string pick(&string key) lives self, key { ... }`.
   - B. A method result borrows from `self` unless it says otherwise; `lives key`
     then means "from `key` instead".
   - C. Original: say where it comes from with a clause word, `from self`/`from key`...
     ```cpp
     &string pick(&string key) lives self, key { ... }     // A
     &string pick(&string key) lives key { ... }          // B (self not included)
     &string pick(&string key) lives from self or key { ... } // C
     ```
