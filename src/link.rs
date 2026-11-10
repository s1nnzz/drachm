//! Turns an object file into an executable by linking it with the runtime,
//! using the system C compiler.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

const RUNTIME_C: &str = include_str!("../runtime/runtime.c");

pub fn link(obj: &[u8], out: &Path, opt: bool) -> Result<(), String> {
    static N: AtomicU32 = AtomicU32::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "drachm-build-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let result = (|| {
        let obj_path = tmp.join("prog.o");
        let rt_path = tmp.join("runtime.c");
        std::fs::write(&obj_path, obj).map_err(|e| e.to_string())?;
        std::fs::write(&rt_path, RUNTIME_C).map_err(|e| e.to_string())?;
        let status = Command::new("cc")
            .arg(&obj_path)
            .arg(&rt_path)
            .arg(if opt { "-O2" } else { "-O0" })
            .arg("-o")
            .arg(out)
            .status();
        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(_) => Err("linking failed".to_string()),
            Err(e) => Err(format!("could not run `cc` (a C compiler is needed for linking): {e}")),
        }
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}
