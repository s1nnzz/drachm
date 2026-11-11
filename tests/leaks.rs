//! The string runtime's own checks, driven from C: with `DRACHM_CHECK_LEAKS`
//! set, freeing a buffer twice aborts with a message, and a leak is reported
//! at exit. (The compiler never makes a program that does this; the check is
//! the safety net under drop elaboration.)

use std::path::Path;
use std::process::Command;

const C: &str = r#"
#include <stdint.h>
#include <string.h>
#include <stdlib.h>
#include "strings.h"

/* The runtime calls this to panic; runtime.c has the real one. */
void drachm_panic(const char *msg) { abort(); }

int main(int argc, char **argv) {
    drachm_str lit = {(uint8_t *)"hi", 2, 0};
    drachm_str a, b;
    drachm_str_concat(&a, &lit, &lit);
    if (strcmp(argv[1], "double") == 0) {
        b = a;
        drachm_str_drop(&a);
        drachm_str_drop(&b);
    } else if (strcmp(argv[1], "leak") == 0) {
        /* a is never dropped */
    } else {
        drachm_str_append(&a, &lit);
        drachm_str_append(&a, &a);
        drachm_str_drop(&a);
    }
    return 0;
}
"#;

fn build() -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("leaks");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.c"), C).unwrap();
    std::fs::write(dir.join("strings.h"), include_str!("../runtime/strings.h")).unwrap();
    let exe = drachm::link::exe_path(&dir.join("t"));
    let out = Command::new(drachm::link::CC)
        .current_dir(&dir)
        .args(drachm::link::CC_FLAGS)
        .args(["t.c", env!("DRACHM_STRINGS_LIB")])
        .args(drachm::link::LINK_FLAGS)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("could not run the C compiler");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    exe
}

fn run(exe: &Path, mode: &str, check: bool) -> (Option<i32>, String) {
    let mut cmd = Command::new(exe);
    cmd.arg(mode).env_remove("DRACHM_CHECK_LEAKS").stdin(std::process::Stdio::null());
    if check {
        cmd.env("DRACHM_CHECK_LEAKS", "1");
    }
    let out = cmd.output().unwrap();
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn runtime_checks() {
    let exe = build();
    let (code, err) = run(&exe, "double", true);
    // abort() is a signal on Unix; on Windows it is a fast fail, which ends
    // the process with STATUS_STACK_BUFFER_OVERRUN.
    assert_eq!(code, if cfg!(windows) { Some(0xC0000409_u32 as i32) } else { None }, "aborted");
    assert!(err.contains("drachm: double free of a string"), "{err}");

    let (code, err) = run(&exe, "leak", true);
    assert_eq!(code, Some(0));
    assert!(err.contains("drachm: leaked 1 strings"), "{err}");

    // Growing in place (realloc) and dropping is not a double free.
    let (code, err) = run(&exe, "fine", true);
    assert_eq!((code, err.as_str()), (Some(0), ""));

    // Without the variable nothing is checked or printed.
    let (code, err) = run(&exe, "leak", false);
    assert_eq!((code, err.as_str()), (Some(0), ""));
}
