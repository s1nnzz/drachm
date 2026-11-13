use super::{DrachmStr, emit, text};

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_os_platform(out: *mut DrachmStr) {
    unsafe { emit(out, std::env::consts::OS.as_bytes()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_os_get_env(key: *const DrachmStr, out: *mut DrachmStr) -> u8 {
    match std::env::var(unsafe { text(key) }) {
        Ok(value) => {
            unsafe { emit(out, value.as_bytes()) };
            1
        }
        Err(_) => 0,
    }
}
