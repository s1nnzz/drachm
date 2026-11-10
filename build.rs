//! Builds the string runtime (`runtime/strings`, a Rust staticlib) and tells
//! the compiler where the archive is, so that `src/link.rs` can embed it.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let target_dir = out_dir.join("strings-target");
    let manifest = "runtime/strings/Cargo.toml";

    println!("cargo:rerun-if-changed={manifest}");
    println!("cargo:rerun-if-changed=runtime/strings/Cargo.lock");
    println!("cargo:rerun-if-changed=runtime/strings/src");
    println!("cargo:rerun-if-changed=runtime/strings.h");
    println!("cargo:rerun-if-changed=build.rs");

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "--release", "--locked", "--manifest-path", manifest, "--target-dir"])
        .arg(&target_dir)
        .status()
        .expect("could not run cargo to build runtime/strings");
    assert!(status.success(), "building runtime/strings failed");

    let archive = target_dir.join("release").join("libdrachm_strings.a");
    assert!(archive.exists(), "{} was not produced", archive.display());
    println!("cargo:rustc-env=DRACHM_STRINGS_LIB={}", archive.display());
}
