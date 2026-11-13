use super::{DrachmStr, emit, text};

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_os_platform(out: *mut DrachmStr) {
    unsafe { emit(out, std::env::consts::OS.as_bytes()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_c_ptr(s: *const DrachmStr) -> *const u8 {
    let bytes = unsafe { super::bytes(s) };
    if bytes.iter().position(|&b| b == 0) != Some(bytes.len().wrapping_sub(1)) {
        super::panic_with(c"text passed to C contains a NUL byte");
    }
    bytes.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_c_text(p: *const u8, out: *mut DrachmStr) -> u8 {
    if p.is_null() {
        return 1;
    }
    let bytes = unsafe { std::ffi::CStr::from_ptr(p as *const std::ffi::c_char) }.to_bytes();
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            unsafe { emit(out, text.as_bytes()) };
            0
        }
        Err(_) => 1,
    }
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
