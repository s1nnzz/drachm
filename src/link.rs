//! Turns compiler output into an executable with the system C compiler:
//! either an object file linked with the runtime, or generated C.

use std::collections::hash_map::DefaultHasher;
use std::ffi::OsStr;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

const RUNTIME_C: &str = include_str!("../runtime/runtime.c");
/// The string runtime's interface, which `runtime.c` includes.
const STRINGS_H: &str = include_str!("../runtime/strings.h");
/// The string runtime, a Rust staticlib built by build.rs.
const STRINGS_LIB: &[u8] = include_bytes!(env!("DRACHM_STRINGS_LIB"));

/// The C compiler. On Windows it is clang, which finds the MSVC linker and
/// libraries by itself.
pub const CC: &str = if cfg!(windows) { "clang" } else { "cc" };
/// Flags for every C compiler run. On Windows: the DLL C runtime, the one the
/// Rust string library uses, so that the program has a single C runtime.
pub const CC_FLAGS: &[&str] = if cfg!(windows) { &["-fms-runtime-lib=dll"] } else { &[] };
/// The end of every link: system libraries the string runtime's Rust standard
/// library needs (as `rustc --print native-static-libs` lists them), then the
/// maths library. On Windows clang also asks for the static C runtime when
/// linking, whatever CC_FLAGS say, so that request is turned off.
pub const LINK_FLAGS: &[&str] = if cfg!(windows) {
    &[
        "-Wl,-nodefaultlib:libcmt",
        "-lkernel32",
        "-lntdll",
        "-luserenv",
        "-lws2_32",
        "-ldbghelp",
        "-llegacy_stdio_definitions",
    ]
} else {
    &["-lpthread", "-ldl", "-lgcc_s", "-lm"]
};
/// The file name ending the linker recognises archives by.
const ARCHIVE_EXT: &str = if cfg!(windows) { "lib" } else { "a" };

/// The file `out` really names: on Windows, a path with no extension gets
/// `.exe`, since Windows runs nothing without one.
pub fn exe_path(out: &Path) -> PathBuf {
    if cfg!(windows) && out.extension().is_none() { out.with_extension("exe") } else { out.to_path_buf() }
}

/// A fresh private directory for intermediate files.
fn scratch_dir() -> Result<PathBuf, String> {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("drachm-build-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn cc(args: &[&OsStr], what: &str) -> Result<(), String> {
    match Command::new(CC).args(CC_FLAGS).args(args).output() {
        Ok(o) if o.status.success() => Ok(()),
        // MSVC's linker reports on stdout.
        Ok(o) => Err(format!("{what} failed:\n{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))),
        Err(e) => Err(format!("could not run `{CC}` (a C compiler is needed): {e}")),
    }
}

/// The user's cache directory for compiled runtime pieces, if there is one.
fn cache_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    };
    base.map(|d| d.join("drachm"))
}

/// The string runtime archive, written once to the cache directory and keyed
/// by its bytes. Published with a rename so that parallel builds never see
/// half a file; if the cache is unusable, a copy in the scratch directory is
/// used instead.
fn strings_archive(scratch: &Path) -> Result<PathBuf, String> {
    let mut h = DefaultHasher::new();
    STRINGS_LIB.hash(&mut h);
    let name = format!("strings-{:016x}.{ARCHIVE_EXT}", h.finish());
    let cache = cache_dir();
    if let Some(dir) = &cache
        && dir.join(&name).exists()
    {
        return Ok(dir.join(&name));
    }

    if let Some(dir) = cache
        && std::fs::create_dir_all(&dir).is_ok()
    {
        let staged = dir.join(format!("{name}.{}", std::process::id()));
        if std::fs::write(&staged, STRINGS_LIB).is_ok() && std::fs::rename(&staged, dir.join(&name)).is_ok() {
            return Ok(dir.join(&name));
        }
        let _ = std::fs::remove_file(&staged);
    }
    let path = scratch.join(&name);
    std::fs::write(&path, STRINGS_LIB).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The runtime compiled to an object file. Compiled once and kept in the
/// user's cache directory, keyed by the runtime's source.
fn runtime_object(scratch: &Path) -> Result<PathBuf, String> {
    let mut h = DefaultHasher::new();
    RUNTIME_C.hash(&mut h);
    STRINGS_H.hash(&mut h);
    env!("CARGO_PKG_VERSION").hash(&mut h);
    let name = format!("runtime-{:016x}.o", h.finish());
    let cache = cache_dir();
    if let Some(dir) = &cache
        && dir.join(&name).exists()
    {
        return Ok(dir.join(&name));
    }

    let src = scratch.join("runtime.c");
    let obj = scratch.join("runtime.o");
    std::fs::write(&src, RUNTIME_C).map_err(|e| e.to_string())?;
    std::fs::write(scratch.join("strings.h"), STRINGS_H).map_err(|e| e.to_string())?;
    cc(&["-O2".as_ref(), "-ffp-contract=off".as_ref(), "-c".as_ref(), src.as_ref(), "-o".as_ref(), obj.as_ref()], "compiling the runtime")?;
    // Publish with a rename so that parallel builds never see half a file.
    // If the cache is unusable, the copy in the scratch directory still works.
    if let Some(dir) = cache
        && std::fs::create_dir_all(&dir).is_ok()
    {
        let staged = dir.join(format!("{name}.{}", std::process::id()));
        if std::fs::copy(&obj, &staged).is_ok() && std::fs::rename(&staged, dir.join(&name)).is_ok() {
            return Ok(dir.join(&name));
        }
        let _ = std::fs::remove_file(&staged);
    }
    Ok(obj)
}

/// Link an object file from the Cranelift backend with the runtime.
pub fn link_object(obj: &[u8], out: &Path) -> Result<(), String> {
    let scratch = scratch_dir()?;
    let result = (|| {
        let obj_path = scratch.join("prog.o");
        std::fs::write(&obj_path, obj).map_err(|e| e.to_string())?;
        let runtime = runtime_object(&scratch)?;
        let strings = strings_archive(&scratch)?;
        let out = exe_path(out);
        let mut args: Vec<&OsStr> = vec![obj_path.as_ref(), runtime.as_ref(), strings.as_ref()];
        args.extend(LINK_FLAGS.iter().map(OsStr::new));
        args.extend([OsStr::new("-o"), out.as_ref()]);
        cc(&args, "linking")
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// Compile C from the optimising backend. Floating point stays strict IEEE:
/// no fast-math, and no fusing of multiply-add, so results match plain builds.
pub fn compile_c(source: &str, out: &Path) -> Result<(), String> {
    let scratch = scratch_dir()?;
    let result = (|| {
        let src = scratch.join("prog.c");
        std::fs::write(&src, source).map_err(|e| e.to_string())?;
        let strings = strings_archive(&scratch)?;
        let out = exe_path(out);
        let mut args: Vec<&OsStr> = ["-O3", "-ffp-contract=off", "-w"].iter().map(OsStr::new).collect();
        args.extend([src.as_os_str(), strings.as_os_str()]);
        args.extend(LINK_FLAGS.iter().map(OsStr::new));
        args.extend([OsStr::new("-o"), out.as_ref()]);
        cc(&args, "compiling the generated C")
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}
