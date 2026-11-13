# Step 7.1 — C header imports

Draft for owner review. Start after Step 7, before Steps 7.6 and 7.5.
See [PLAN](../PLAN.md), [SPEC](../SPEC.md), and
[owner answers](owner-answers.md) under “C bindings and library checks”.

## Decisions already made

```cpp
in * from c."SDL2/SDL.h" as sdl;
```

Use libclang via `clang-sys` to read C headers. Direct C calls require
Drachm's existing unsafe context. A Drachm wrapper can expose a safe
signature and keep the unsafe operations inside its implementation.
The wrapper author establishes the C preconditions and handles ownership.
A C declaration alone supplies no proof of either. Native libraries are
declared in `drachm.toml`. The manifest schema is open below. A library
project can be checked without `main()`.

## Implementation packages

1. **Import and discovery.** Extend the Step 7 import AST with a C header
   source. Preserve the header and alias spans. Resolve it through a foreign
   namespace in the existing per-file name resolver, without adding a
   Drachm source file to the module graph. Load libclang only for C imports.
   Parse a synthetic C translation unit with the selected header and the
   effective target, include paths and preprocessing settings. Retain
   declaration provenance and libclang diagnostics. Cache discovery per
   compilation by header and effective compiler settings. Distinguish a
   missing libclang, an unreadable header, a C parse error and an unknown
   name. Record known but unsupported declarations so use can report why
   the declaration is unavailable.
2. **Checking and lowering.** Keep C symbol identity, canonical types and
   calling convention in a shared signature table. Resolve alias-qualified
   calls to that table and check unsafe context and arguments before lowering.
   Lower to an explicit foreign call in HIR and MIR, preserving source span
   and signature. Foreign calls are conservatively effectful. Preserve C
   signedness and width from the target, including plain `char` and
   `long`, and verify narrow integer ABI extension and return handling.
   Start by validating the fixed-arity scalar path on both backends. Gate
   other signatures on the owner decisions and verified ABI support.
3. **Code generation and linking.** Use a shared foreign signature and link
   plan for `src/codegen_mir.rs`, `src/cgen_mir.rs` and `src/link.rs`.
   Cranelift declares external symbols with the target C ABI. Generated C
   sees compatible prototypes with the original symbol names. Any future
   header-dependent C shim must serve both backends and use the same
   target and preprocessing settings as discovery. Validate the ABI
   before emission, including the calling convention. Reject unsupported
   signatures in the shared frontend. Pass manifest inputs as individual
   driver arguments, preserving native library order after objects that
   reference them. Resolve configured paths relative to the declaring
   manifest once the schema is approved. Use existing scratch cleanup
   for generated artifacts on success and failure. `drachm check` performs
   discovery and checking without linking. Include the missing symbol or
   library and the relevant manifest location in link failure reports when
   available.
4. **Acceptance and documentation.** Use a repository-owned C fixture and
   header so parity tests need no SDL installation. Cover aliased calls
   across modules, unsafe rejection and safe wrapper use, ABI boundary
   values, symbol collisions, missing headers and libraries, an unsupported
   declaration alongside a usable one, and library checks without
   `main()`. Exercise clean builds and error paths for both backends.
   For approved resource wrappers, count allocations and releases through
   success, failure and early return. Document required toolchain components
   and how missing dependencies are surfaced. In the feature commit update
   the site tour/reference, add a tested example with expected output,
   remove status markers for the shipped slice, and integrate the feature
   into `examples/showcase/`. Regenerate the error reference if diagnostic
   codes change. Run targeted tests with
   `nice -n 19 env CARGO_BUILD_JOBS=3 DRACHM_TEST_JOBS=2`, then run the full
   `cargo test` once in the foreground with those limits.

## Owner review

Options below are proposals. Where a choice affects accepted syntax,
name visibility, types or ownership, use E0901 for the affected construct
until the owner decides. Use E0901 for recognised, unimplemented ABI
shapes too. Report each interim in the implementation handoff.

### Batch A — Imports and build inputs

The import exposes declarations from the named header and its transitive
includes. Repeated declarations of the same C symbol resolve to one
declaration in the import. A flat `[native]` table holds include directories,
definitions and libraries. Scripts can check headers using default search
paths. Linking requires a project manifest.

Still open: platform-specific settings, local archives, and whether library
consumers inherit native link settings from a dependency.

### Batch B — Types and layout

1. **Scalars.** Use target-sized Drachm primitives, introduce distinct C
   scalar types, or keep C spelling as a type annotation that preserves the
   ABI while requiring explicit conversion at the wrapper boundary? Decide
   the handling of plain `char`, C booleans and types with no equivalent
   Drachm representation.
2. **Pointers.** Map qualifiers onto existing `*T` and `*fixed T`, use
   opaque handles for incomplete pointees with raw pointers elsewhere, or
   expose foreign pointers through header-scoped types that can be
   explicitly opened inside unsafe code? Decide how to handle `void *`,
   nulls and pointer-to-pointer out parameters.
3. **Structs and unions.** Start with opaque pointer access through C
   functions, expose verified C layouts and by-value calls, or support
   named field projections through generated C accessors before allowing
   aggregate values? Decide separately whether unions expose unsafe
   member access and how bit-fields and flexible array members are handled.

### Batch C — Values and callable declarations

1. **Enums.** Expose ABI integers and named constants, expose a foreign
   enum type that admits unknown values, or use an explicit checked wrapper
   conversion that returns the original integer when no case matches?
2. **Macros and inline functions.** Import evaluated object-like constants
   only, also admit inline functions through typed C shims, or let a
   wrapper name a macro and declare its typed argument and result contract?
   Decide which forms of macro expansion, if any, can be called.
3. **Callbacks and variadics.** For callbacks, choose plain C-ABI functions,
   explicit context-pointer adapters, or a scoped registration construct that
   makes the wrapper responsible for deregistration before the scope ends.
   For variadics, defer them, allow explicit promoted argument types, or use
   wrapper-declared fixed call shapes. Decide independently whether each
   belongs in 7.1. Callback retention, foreign threads and unwinding need
   decisions before accepting those cases.

### Batch D — Safe wrappers and resource contracts

1. **Strings and buffers.** Keep raw pointer and length conversion in
   wrappers, offer scoped conversion helpers with explicit encoding and
   termination, or allow a wrapper to declare a buffer contract naming the
   length source and whether the buffer can escape? Decide who handles
   embedded NUL, invalid encoding and nullable results.
2. **Returned memory.** Require wrappers to pair allocation and release
   manually, allow wrappers to bind an owning handle to a named C release
   function, or offer a scoped acquisition form that releases on every
   exit? Decide how wrappers represent borrowed results tied to an input
   or valid only until the next C call. Never infer a release function or
   give a C allocation to Drachm's allocator based on its pointer type.
3. **Failure conversion.** Leave error translation to ordinary wrapper code,
   provide reusable checks for common sentinel conventions, or let wrappers
   declare a named success predicate and a failure reader that runs
   immediately after the call? Decide the first fixture's contract so
   error reporting and cleanup can be tested together.
