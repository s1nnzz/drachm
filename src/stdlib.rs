//! The standard library's own source (step 7 WP6, engineering choice E3).
//!
//! A standard module is Drachm source embedded in the compiler (`src/stdlib/NAME.drh`,
//! `include_str!`). `in NAME;` loads it into the module graph like a project file
//! (shown in diagnostics as `std/NAME.drh`), so its names resolve, export and mangle
//! like any other module's (`math.sqrt`).
//!
//! The few operations that must reach the runtime are written as **intrinsic**
//! functions: `out float64 sqrt(float64 x) intrinsic;`. An intrinsic has no body. The
//! resolver keeps it out of the program and records its name; the checker types a call
//! of it by its own rule (`typeck/math.rs`, `typeck/library.rs`), which is how one
//! declaration can serve every number type and every unit. The declared types are
//! documentation and the arity. `intrinsic` is accepted by the parser everywhere but
//! only the embedded modules may use it: a user file that does gets E0243.
//!
//! `io` (its formatting reads the format string at compile time) and `si` (a unit
//! prelude, `src/prelude/si.drh`) stay special in the checker.

/// The embedded modules written in Drachm: name and source.
pub const LIBS: &[(&str, &str)] = &[
    ("math", include_str!("stdlib/math.drh")),
    ("os", include_str!("stdlib/os.drh")),
    ("fs", include_str!("stdlib/fs.drh")),
    ("time", include_str!("stdlib/time.drh")),
    ("string", include_str!("stdlib/string.drh")),
    ("convert", include_str!("stdlib/convert.drh")),
];

/// The modules built into the compiler that are not embedded source.
pub const SPECIAL: &[&str] = &["io", "si"];

/// The source of embedded module `name`.
pub fn source(name: &str) -> Option<&'static str> {
    LIBS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// Every standard module name.
pub fn names() -> impl Iterator<Item = &'static str> {
    SPECIAL.iter().copied().chain(LIBS.iter().map(|(n, _)| *n))
}

/// Is `name` a standard module (embedded or special)?
pub fn is_module(name: &str) -> bool {
    names().any(|n| n == name)
}
