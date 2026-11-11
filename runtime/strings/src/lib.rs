//! The Drachm string runtime. The C interface is declared in
//! `runtime/strings.h`; this crate implements it.
//!
//! Strings are UTF-8 `{ ptr, len, cap }` triples. Owned buffers come from libc
//! `malloc` (never Rust's allocator) because the C side of the runtime
//! allocates and frees them too. "Character" means grapheme cluster.
//!
//! The exported functions use the `C-unwind` ABI only so that unit tests can
//! observe panics; with `panic = "abort"` it behaves exactly like `C`.

#![allow(clippy::missing_safety_doc)] // every pointer argument is specified by runtime/strings.h

use std::cmp::Ordering;
use std::ffi::c_char;
use std::ptr;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Mutex, Once};

use unicode_segmentation::UnicodeSegmentation;

#[repr(C)]
pub struct DrachmStr {
    ptr: *mut u8,
    len: u64,
    cap: u64,
}

const EMPTY: DrachmStr = DrachmStr { ptr: ptr::null_mut(), len: 0, cap: 0 };

// ---- panics ---------------------------------------------------------------

#[cfg(not(test))]
unsafe extern "C-unwind" {
    /// Provided by the C runtime.
    fn drachm_panic(msg: *const c_char) -> !;
}

/// Test stand-in for the C runtime's `drachm_panic`.
#[cfg(test)]
unsafe extern "C-unwind" fn drachm_panic(msg: *const c_char) -> ! {
    let msg = unsafe { std::ffi::CStr::from_ptr(msg) };
    panic!("{}", msg.to_string_lossy());
}

fn panic_with(msg: &'static std::ffi::CStr) -> ! {
    unsafe { drachm_panic(msg.as_ptr()) }
}

fn index_panic() -> ! {
    panic_with(c"string index out of range")
}

// ---- allocation and the live counter ----------------------------------------

static LIVE: AtomicU64 = AtomicU64::new(0);
static EXIT_HOOK: Once = Once::new();

/// `DRACHM_CHECK_LEAKS` is set: also keep the set of live owned buffers, to
/// catch a buffer freed or reallocated twice. Read once, at the first
/// allocation; without the variable nothing but this flag is ever looked at.
static CHECKING: AtomicBool = AtomicBool::new(false);
static OWNED: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

#[inline]
fn checking() -> bool {
    CHECKING.load(AtomicOrdering::Relaxed)
}

fn track(p: *mut u8) {
    OWNED.lock().unwrap().get_or_insert_with(HashSet::new).insert(p as usize);
}

/// Forget a buffer that is about to be freed or moved; abort if it is not one
/// that is live.
fn untrack(p: *mut u8) {
    let known = OWNED.lock().unwrap().as_mut().is_some_and(|set| set.remove(&(p as usize)));
    if !known {
        eprintln!("drachm: double free of a string");
        unsafe { libc::abort() };
    }
}

extern "C" fn report_leaks() {
    let live = LIVE.load(AtomicOrdering::Relaxed);
    if live != 0 && std::env::var_os("DRACHM_CHECK_LEAKS").is_some() {
        eprintln!("drachm: leaked {live} strings");
    }
}

/// Allocate an owned buffer of `cap` bytes and count it. `cap == 0` gives NULL.
#[unsafe(no_mangle)]
pub extern "C-unwind" fn drachm_str_alloc(cap: u64) -> *mut u8 {
    if cap == 0 {
        return ptr::null_mut();
    }
    EXIT_HOOK.call_once(|| {
        CHECKING.store(std::env::var_os("DRACHM_CHECK_LEAKS").is_some(), AtomicOrdering::Relaxed);
        unsafe { libc::atexit(report_leaks) };
    });
    let p = unsafe { libc::malloc(cap as usize) } as *mut u8;
    if p.is_null() {
        panic_with(c"out of memory");
    }
    if checking() {
        track(p);
    }
    LIVE.fetch_add(1, AtomicOrdering::Relaxed);
    p
}

#[unsafe(no_mangle)]
pub extern "C-unwind" fn drachm_str_live() -> u64 {
    LIVE.load(AtomicOrdering::Relaxed)
}

// ---- views and constructors -------------------------------------------------

/// The bytes of a string; an empty string may have a NULL pointer.
unsafe fn bytes<'a>(s: *const DrachmStr) -> &'a [u8] {
    let s = unsafe { &*s };
    if s.len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(s.ptr, s.len as usize) } }
}

/// Strings are always valid UTF-8 (the header's contract).
unsafe fn text<'a>(s: *const DrachmStr) -> &'a str {
    unsafe { std::str::from_utf8_unchecked(bytes(s)) }
}

/// Write an owned copy of `data` to `out` (empty data gives the empty string).
unsafe fn emit(out: *mut DrachmStr, data: &[u8]) {
    let p = drachm_str_alloc(data.len() as u64);
    if !p.is_null() {
        unsafe { ptr::copy_nonoverlapping(data.as_ptr(), p, data.len()) };
    }
    unsafe { out.write(DrachmStr { ptr: p, len: data.len() as u64, cap: data.len() as u64 }) };
}

// ---- lifetime ---------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_drop(s: *mut DrachmStr) {
    let s = unsafe { &mut *s };
    if s.cap > 0 {
        if checking() {
            untrack(s.ptr);
        }
        unsafe { libc::free(s.ptr as *mut _) };
        LIVE.fetch_sub(1, AtomicOrdering::Relaxed);
    }
    *s = EMPTY;
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_copy(out: *mut DrachmStr, s: *const DrachmStr) {
    let src = unsafe { &*s };
    if src.cap == 0 {
        // Not owned, so the copy may share the same read-only bytes.
        unsafe { out.write(DrachmStr { ..*src }) };
    } else {
        unsafe { emit(out, bytes(s)) };
    }
}

/// Write an owned string made of `parts` laid end to end, copying straight
/// into the new buffer.
unsafe fn emit_parts(out: *mut DrachmStr, parts: &[&[u8]]) {
    let total = parts.iter().try_fold(0usize, |n, p| n.checked_add(p.len())).filter(|&n| n <= isize::MAX as usize);
    let Some(total) = total else { panic_with(c"string too long") };
    let p = drachm_str_alloc(total as u64);
    let mut at = 0;
    for part in parts.iter().filter(|p| !p.is_empty()) {
        unsafe { ptr::copy_nonoverlapping(part.as_ptr(), p.add(at), part.len()) };
        at += part.len();
    }
    unsafe { out.write(DrachmStr { ptr: p, len: total as u64, cap: total as u64 }) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_concat(out: *mut DrachmStr, a: *const DrachmStr, b: *const DrachmStr) {
    unsafe { emit_parts(out, &[bytes(a), bytes(b)]) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_join(out: *mut DrachmStr, parts: *const DrachmStr, n: u64) {
    let parts = if n == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(parts, n as usize) } };
    let slices: Vec<&[u8]> = parts.iter().map(|p| unsafe { bytes(p) }).collect();
    unsafe { emit_parts(out, &slices) };
}

/// Only raw pointer reads and writes here: `t` may be the same string as `s`.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_append(s: *mut DrachmStr, t: *const DrachmStr) {
    let (sp, slen, scap) = unsafe { ((*s).ptr, (*s).len, (*s).cap) };
    let (tp, tlen) = unsafe { ((*t).ptr, (*t).len) };
    if tlen == 0 {
        return;
    }
    let Some(len) = slen.checked_add(tlen).filter(|&n| n <= isize::MAX as u64) else { panic_with(c"string too long") };
    let (buf, cap) = if scap >= len {
        (sp, scap)
    } else if scap == 0 {
        // Not owned: start an owned buffer; both inputs stay readable meanwhile.
        let cap = len.max(16);
        let p = drachm_str_alloc(cap);
        if slen > 0 {
            unsafe { ptr::copy_nonoverlapping(sp, p, slen as usize) };
        }
        (p, cap)
    } else {
        let cap = len.max(scap.saturating_mul(2)).min(isize::MAX as u64);
        if checking() {
            untrack(sp);
        }
        let p = unsafe { libc::realloc(sp as *mut _, cap as usize) } as *mut u8;
        if p.is_null() {
            panic_with(c"out of memory");
        }
        if checking() {
            track(p);
        }
        (p, cap)
    };
    // When `t` lives in `s`'s buffer, its bytes moved along with it.
    let inside = scap > 0 && tp >= sp && (tp as usize) < sp as usize + slen as usize;
    let src = if inside { unsafe { buf.add(tp as usize - sp as usize) } } else { tp };
    unsafe {
        ptr::copy(src, buf.add(slen as usize), tlen as usize);
        s.write(DrachmStr { ptr: buf, len, cap });
    }
}

// ---- comparison -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_eq(a: *const DrachmStr, b: *const DrachmStr) -> u8 {
    unsafe { (bytes(a) == bytes(b)) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_cmp(a: *const DrachmStr, b: *const DrachmStr) -> i32 {
    match unsafe { bytes(a).cmp(bytes(b)) } {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

// ---- character helpers ------------------------------------------------------

/// Is every character one byte? True for ASCII without a CR LF pair, the one
/// ASCII grapheme cluster longer than a byte. Such text skips segmentation.
fn bytewise(s: &str) -> bool {
    s.is_ascii() && !s.contains("\r\n")
}

fn char_count(s: &str) -> usize {
    if bytewise(s) { s.len() } else { s.graphemes(true).count() }
}

/// Byte offsets where each character starts, followed by the string's length,
/// so character `i` spans `offsets[i]..offsets[i + 1]`.
fn char_offsets(s: &str) -> Vec<usize> {
    let mut v: Vec<usize> = s.grapheme_indices(true).map(|(i, _)| i).collect();
    v.push(s.len());
    v
}

/// Byte offset where character `i` starts (`i` may be the count: the end).
fn char_start(s: &str, i: usize) -> usize {
    if bytewise(s) { i } else { s.grapheme_indices(true).nth(i).map_or(s.len(), |(at, _)| at) }
}

/// Resolve a possibly negative index against `count` characters. With
/// `allow_end`, `count` itself is valid (a position between characters).
fn resolve(index: i64, count: usize, allow_end: bool) -> usize {
    let i = if index < 0 { index.checked_add(count as i64) } else { Some(index) };
    match i {
        Some(i) if i >= 0 && (i as u64) < count as u64 + allow_end as u64 => i as usize,
        _ => index_panic(),
    }
}

/// Byte range of the inclusive character range `start..=end`.
fn char_range(s: &str, start: i64, end: i64) -> (usize, usize) {
    if bytewise(s) {
        let (a, b) = (resolve(start, s.len(), false), resolve(end, s.len(), false));
        if a > b {
            index_panic();
        }
        return (a, b + 1);
    }
    let offs = char_offsets(s);
    let count = offs.len() - 1;
    let (a, b) = (resolve(start, count, false), resolve(end, count, false));
    if a > b {
        index_panic();
    }
    (offs[a], offs[b + 1])
}

// ---- methods ----------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_len(s: *const DrachmStr) -> u64 {
    unsafe { char_count(text(s)) as u64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_is_empty(s: *const DrachmStr) -> u8 {
    unsafe { ((*s).len == 0) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_reverse(out: *mut DrachmStr, s: *const DrachmStr) {
    let t = unsafe { text(s) };
    let mut v = Vec::with_capacity(t.len());
    if bytewise(t) {
        v.extend(t.bytes().rev());
    } else {
        for g in t.graphemes(true).rev() {
            v.extend_from_slice(g.as_bytes());
        }
    }
    unsafe { emit(out, &v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_sub(out: *mut DrachmStr, s: *const DrachmStr, start: i64, end: i64) {
    let t = unsafe { text(s) };
    let (a, b) = char_range(t, start, end);
    unsafe { emit(out, &t.as_bytes()[a..b]) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_upper(out: *mut DrachmStr, s: *const DrachmStr) {
    unsafe { emit(out, text(s).to_uppercase().as_bytes()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_lower(out: *mut DrachmStr, s: *const DrachmStr) {
    unsafe { emit(out, text(s).to_lowercase().as_bytes()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_trim(out: *mut DrachmStr, s: *const DrachmStr) {
    unsafe { emit(out, text(s).trim().as_bytes()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_find(s: *const DrachmStr, pat: *const DrachmStr) -> i64 {
    let t = unsafe { text(s) };
    let Some(at) = t.find(unsafe { text(pat) }) else { return -1 };
    if bytewise(t) {
        return at as i64;
    }
    // A match that starts inside a character belongs to that character.
    t.grapheme_indices(true).take_while(|&(i, _)| i <= at).count().saturating_sub(1) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_contains(s: *const DrachmStr, pat: *const DrachmStr) -> u8 {
    unsafe { text(s).contains(text(pat)) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_starts_with(s: *const DrachmStr, pat: *const DrachmStr) -> u8 {
    unsafe { bytes(s).starts_with(bytes(pat)) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_ends_with(s: *const DrachmStr, pat: *const DrachmStr) -> u8 {
    unsafe { bytes(s).ends_with(bytes(pat)) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_count(s: *const DrachmStr, pat: *const DrachmStr) -> u64 {
    let p = unsafe { text(pat) };
    if p.is_empty() { 0 } else { unsafe { text(s).matches(p).count() as u64 } }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_replace(
    out: *mut DrachmStr,
    s: *const DrachmStr,
    from: *const DrachmStr,
    to: *const DrachmStr,
) {
    unsafe { drachm_str_replace_limit(out, s, from, to, u64::MAX) };
}

/// Replace the first `n` non-overlapping matches. An empty `from` matches
/// nowhere, so the string is copied unchanged.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_replace_limit(
    out: *mut DrachmStr,
    s: *const DrachmStr,
    from: *const DrachmStr,
    to: *const DrachmStr,
    n: u64,
) {
    let (t, f, r) = unsafe { (text(s), text(from), text(to)) };
    if f.is_empty() || n == 0 {
        unsafe { emit(out, t.as_bytes()) };
    } else {
        let limit = usize::try_from(n).unwrap_or(usize::MAX);
        unsafe { emit(out, t.replacen(f, r, limit).as_bytes()) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_insert(out: *mut DrachmStr, s: *const DrachmStr, index: i64, ins: *const DrachmStr) {
    let t = unsafe { text(s) };
    let at = char_start(t, resolve(index, char_count(t), true));
    let mut v = Vec::with_capacity(t.len() + unsafe { (*ins).len } as usize);
    v.extend_from_slice(&t.as_bytes()[..at]);
    v.extend_from_slice(unsafe { bytes(ins) });
    v.extend_from_slice(&t.as_bytes()[at..]);
    unsafe { emit(out, &v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_remove(out: *mut DrachmStr, s: *const DrachmStr, start: i64, end: i64) {
    let t = unsafe { text(s) };
    let (a, b) = char_range(t, start, end);
    let mut v = Vec::with_capacity(t.len() - (b - a));
    v.extend_from_slice(&t.as_bytes()[..a]);
    v.extend_from_slice(&t.as_bytes()[b..]);
    unsafe { emit(out, &v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_repeat(out: *mut DrachmStr, s: *const DrachmStr, n: u64) {
    let b = unsafe { bytes(s) };
    let total = (b.len() as u64).checked_mul(n).filter(|&t| t <= isize::MAX as u64);
    let Some(total) = total else { panic_with(c"string too long") };
    let p = drachm_str_alloc(total);
    for i in 0..total as usize / b.len().max(1) {
        unsafe { ptr::copy_nonoverlapping(b.as_ptr(), p.add(i * b.len()), b.len()) };
    }
    unsafe { out.write(DrachmStr { ptr: p, len: total, cap: total }) };
}

/// Shared by padLeft and padRight: the padding text that brings `t` up to `width`
/// characters, or `None` when it is already wide enough.
fn padding(t: &str, width: u64, fill: &str) -> Option<String> {
    if fill.is_empty() {
        panic_with(c"empty padding fill");
    }
    let have = char_count(t) as u64;
    let need = usize::try_from(width.checked_sub(have)?).ok().filter(|&n| n > 0)?;
    // The fill repeats character by character, so the last repeat is cut at a
    // character boundary.
    Some(fill.graphemes(true).cycle().take(need).collect())
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_pad_left(out: *mut DrachmStr, s: *const DrachmStr, width: u64, fill: *const DrachmStr) {
    let t = unsafe { text(s) };
    match padding(t, width, unsafe { text(fill) }) {
        Some(pad) => unsafe { emit(out, [pad.as_bytes(), t.as_bytes()].concat().as_slice()) },
        None => unsafe { emit(out, t.as_bytes()) },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_pad_right(out: *mut DrachmStr, s: *const DrachmStr, width: u64, fill: *const DrachmStr) {
    let t = unsafe { text(s) };
    match padding(t, width, unsafe { text(fill) }) {
        Some(pad) => unsafe { emit(out, [t.as_bytes(), pad.as_bytes()].concat().as_slice()) },
        None => unsafe { emit(out, t.as_bytes()) },
    }
}

// ---- number to text ---------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_radix(out: *mut DrachmStr, lo: u64, hi: u64, is_signed: u8, base: u32, upper: u8) {
    if !(2..=36).contains(&base) {
        panic_with(c"invalid number base");
    }
    let mut v = (hi as u128) << 64 | lo as u128;
    let negative = is_signed != 0 && (hi >> 63) == 1;
    if negative {
        v = v.wrapping_neg();
    }
    let digits: &[u8; 36] = if upper != 0 { b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ" } else { b"0123456789abcdefghijklmnopqrstuvwxyz" };
    // At most 128 digits (base 2) plus the sign.
    let mut buf = [0u8; 129];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = digits[(v % base as u128) as usize];
        v /= base as u128;
        if v == 0 {
            break;
        }
    }
    if negative {
        i -= 1;
        buf[i] = b'-';
    }
    unsafe { emit(out, &buf[i..]) };
}


// ---- input and number text ----------------------------------------------------
//
// Lines of standard input are read by the C runtime (runtime.c), so that both
// backends share one buffer; these turn the bytes into strings and parse them.

/// Bytes of input as a new string. Each invalid UTF-8 sequence becomes U+FFFD.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_from_bytes(out: *mut DrachmStr, p: *const u8, n: u64) {
    let data: &[u8] = if n == 0 { &[] } else { unsafe { std::slice::from_raw_parts(p, n as usize) } };
    match std::str::from_utf8(data) {
        Ok(_) => unsafe { emit(out, data) },
        Err(_) => unsafe { emit(out, String::from_utf8_lossy(data).as_bytes()) },
    }
}

/// Splits an optional leading `-` or `+` off the text. Returns whether it is
/// negative, and the rest.
fn split_sign(t: &str) -> (bool, &str) {
    match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    }
}

/// The grammar for integers: surrounding whitespace is ignored, then an optional
/// sign (`-` only for signed types, `+` for any), then decimal digits. Anything
/// else, and any value out of range for the type, is `None`.
fn parse_int(t: &str, signed: bool, bits: u32) -> Option<(u64, u64)> {
    let (negative, digits) = split_sign(t.trim());
    if (negative && !signed) || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let magnitude: u128 = digits.parse().ok()?;
    let v = if !signed {
        let max = if bits == 128 { u128::MAX } else { (1u128 << bits) - 1 };
        if magnitude > max {
            return None;
        }
        magnitude
    } else {
        let half = 1u128 << (bits - 1);
        let ok = if negative { magnitude <= half } else { magnitude < half };
        if !ok {
            return None;
        }
        if negative { magnitude.wrapping_neg() } else { magnitude }
    };
    Some((v as u64, (v >> 64) as u64))
}

/// The grammar for floats: surrounding whitespace is ignored, then an optional
/// sign (`-` or `+`), digits with an optional `.` and digits after it (`.5` and
/// `5.` are fine, a lone `.` is not), and an optional exponent `e`/`E` with an
/// optional sign and digits. `inf`, `nan` and hex are not numbers, and neither
/// is a value that overflows to infinity.
fn is_float_text(t: &str) -> bool {
    let b = t.trim().as_bytes();
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > start
    };
    let mut i = 0;
    if matches!(b.first(), Some(b'-' | b'+')) {
        i += 1;
    }
    let before = digits(&mut i);
    let mut after = false;
    if b.get(i) == Some(&b'.') {
        i += 1;
        after = digits(&mut i);
    }
    if !before && !after {
        return false;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !digits(&mut i) {
            return false;
        }
    }
    i == b.len()
}

/// Parses the whole of `s` as an integer of the given kind: 1 and the value
/// (`lo`, and `hi` for 128 bits; sign-extended for signed kinds), or 0 for bad text.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_parse_int(s: *const DrachmStr, is_signed: u8, bits: u32, lo: *mut u64, hi: *mut u64) -> u8 {
    match parse_int(unsafe { text(s) }, is_signed != 0, bits) {
        Some((l, h)) => {
            unsafe { *lo = l; *hi = h };
            1
        }
        None => 0,
    }
}

/// Parses the whole of `s` as a float64 (1 and the value, or 0 for bad text).
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_parse_f64(s: *const DrachmStr, out: *mut f64) -> u8 {
    let t = unsafe { text(s) }.trim();
    match t.parse::<f64>() {
        Ok(v) if is_float_text(t) && v.is_finite() => {
            unsafe { *out = v };
            1
        }
        _ => 0,
    }
}

/// As `drachm_str_parse_f64`, for float32.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_parse_f32(s: *const DrachmStr, out: *mut f32) -> u8 {
    let t = unsafe { text(s) }.trim();
    match t.parse::<f32>() {
        Ok(v) if is_float_text(t) && v.is_finite() => {
            unsafe { *out = v };
            1
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"; // one character
    const FLAG: &str = "\u{1F1EF}\u{1F1F5}"; // one character
    const E_ACUTE: &str = "\u{e9}";
    const E_COMBINING: &str = "e\u{301}"; // also one character
const ACUTE: &str = "\u{301}"; // a bare combining mark

    fn lit(s: &str) -> DrachmStr {
        DrachmStr { ptr: s.as_ptr() as *mut u8, len: s.len() as u64, cap: 0 }
    }

    /// Run a function that writes to `out`, return the result as a String and drop it.
    fn run(f: impl FnOnce(*mut DrachmStr)) -> String {
        let mut out = EMPTY;
        f(&mut out);
        let r = String::from_utf8(unsafe { bytes(&out) }.to_vec()).unwrap();
        unsafe { drachm_str_drop(&mut out) };
        r
    }

    fn len(s: &str) -> u64 {
        unsafe { drachm_str_len(&lit(s)) }
    }
    fn sub(s: &str, a: i64, b: i64) -> String {
        run(|o| unsafe { drachm_str_sub(o, &lit(s), a, b) })
    }
    fn remove(s: &str, a: i64, b: i64) -> String {
        run(|o| unsafe { drachm_str_remove(o, &lit(s), a, b) })
    }
    fn insert(s: &str, i: i64, t: &str) -> String {
        run(|o| unsafe { drachm_str_insert(o, &lit(s), i, &lit(t)) })
    }
    fn find(s: &str, t: &str) -> i64 {
        unsafe { drachm_str_find(&lit(s), &lit(t)) }
    }
    fn pad_left(s: &str, w: u64, f: &str) -> String {
        run(|o| unsafe { drachm_str_pad_left(o, &lit(s), w, &lit(f)) })
    }
    fn pad_right(s: &str, w: u64, f: &str) -> String {
        run(|o| unsafe { drachm_str_pad_right(o, &lit(s), w, &lit(f)) })
    }
    fn radix(v: i128, signed: bool, base: u32, upper: bool) -> String {
        let u = v as u128;
        run(|o| unsafe { drachm_str_radix(o, u as u64, (u >> 64) as u64, signed as u8, base, upper as u8) })
    }

    #[test]
    fn length_counts_graphemes() {
        assert_eq!(len(""), 0);
        assert_eq!(len("hello"), 5);
        assert_eq!(len(E_ACUTE), 1);
        assert_eq!(len(E_COMBINING), 1);
        assert_eq!(len(&format!("a{ACUTE}b")), 2);
        assert_eq!(len("\u{1F600}"), 1);
        assert_eq!(len(FAMILY), 1);
        assert_eq!(len(FLAG), 1);
        assert_eq!(len(&format!("{FLAG}{FLAG}")), 2);
        assert_eq!(len("a\r\nb"), 3);
        assert_eq!(len("a\rb\n"), 4);
        assert_eq!(unsafe { drachm_str_is_empty(&lit("")) }, 1);
        assert_eq!(unsafe { drachm_str_is_empty(&EMPTY) }, 1);
        assert_eq!(unsafe { drachm_str_is_empty(&lit("x")) }, 0);
        assert_eq!(unsafe { drachm_str_len(&EMPTY) }, 0);
    }

    #[test]
    fn concat_copy_drop_and_counter() {
        let before = drachm_str_live();
        let (a, b) = (lit("foo"), lit("bar"));
        let mut c = EMPTY;
        unsafe { drachm_str_concat(&mut c, &a, &b) };
        assert!(c.cap >= 6);
        assert_eq!(unsafe { bytes(&c) }, b"foobar");
        let mut d = EMPTY;
        unsafe { drachm_str_copy(&mut d, &c) };
        assert_ne!(d.ptr, c.ptr);
        assert_eq!(unsafe { bytes(&d) }, b"foobar");
        assert!(drachm_str_live() >= before + 2);
        unsafe { drachm_str_drop(&mut c) };
        assert_eq!((c.ptr, c.len, c.cap), (ptr::null_mut(), 0, 0));
        unsafe { drachm_str_drop(&mut c) }; // dropping an empty string is fine
        unsafe { drachm_str_drop(&mut d) };

        // Not-owned strings are shared, and dropping them frees nothing.
        let mut e = EMPTY;
        unsafe { drachm_str_copy(&mut e, &a) };
        assert_eq!((e.ptr, e.cap), (a.ptr, 0));
        unsafe { drachm_str_drop(&mut e) };

        let mut z = EMPTY;
        unsafe { drachm_str_concat(&mut z, &EMPTY, &EMPTY) };
        assert_eq!((z.ptr, z.len, z.cap), (ptr::null_mut(), 0, 0));
        assert!(drachm_str_alloc(0).is_null());
    }

    #[test]
    fn joining() {
        let parts = [lit("ab"), EMPTY, lit(FLAG), lit("")];
        assert_eq!(run(|o| unsafe { drachm_str_join(o, parts.as_ptr(), 4) }), format!("ab{FLAG}"));
        assert_eq!(run(|o| unsafe { drachm_str_join(o, parts.as_ptr(), 1) }), "ab");
        let mut z = EMPTY;
        unsafe { drachm_str_join(&mut z, ptr::null(), 0) };
        assert_eq!((z.ptr, z.len, z.cap), (ptr::null_mut(), 0, 0));
    }

    #[test]
    fn appending() {
        let before = drachm_str_live();
        // Appending to a literal makes it owned; appending nothing changes nothing.
        let mut s = lit("ab");
        unsafe { drachm_str_append(&mut s, &EMPTY) };
        assert_eq!(s.cap, 0);
        unsafe { drachm_str_append(&mut s, &lit("cd")) };
        assert!(s.cap >= 4);
        assert_eq!(unsafe { bytes(&s) }, b"abcd");
        // Appending a string to itself, in place and with a reallocation.
        for _ in 0..4 {
            let sp: *mut DrachmStr = &mut s;
            unsafe { drachm_str_append(sp, sp) };
        }
        assert_eq!(unsafe { bytes(&s) }, "abcd".repeat(16).as_bytes());
        for i in 0..100 {
            unsafe { drachm_str_append(&mut s, &lit(if i % 2 == 0 { "x" } else { E_ACUTE })) };
        }
        assert_eq!(s.len, 64 + 50 + 100);
        // Other tests run in parallel, so only a lower bound is certain.
        assert!(drachm_str_live() > before);
        unsafe { drachm_str_drop(&mut s) };
        let mut e = EMPTY;
        unsafe { drachm_str_append(&mut e, &lit("x")) };
        assert_eq!(unsafe { bytes(&e) }, b"x");
        unsafe { drachm_str_drop(&mut e) };
    }

    #[test]
    fn compare() {
        let (a, b, c) = (lit("abc"), lit("abd"), lit("ab"));
        unsafe {
            assert_eq!(drachm_str_eq(&a, &a), 1);
            assert_eq!(drachm_str_eq(&a, &b), 0);
            assert_eq!(drachm_str_eq(&EMPTY, &lit("")), 1);
            assert_eq!(drachm_str_cmp(&a, &b), -1);
            assert_eq!(drachm_str_cmp(&b, &a), 1);
            assert_eq!(drachm_str_cmp(&a, &a), 0);
            assert_eq!(drachm_str_cmp(&c, &a), -1);
            assert_eq!(drachm_str_cmp(&EMPTY, &c), -1);
        }
        // Precomposed and combining forms differ by bytes.
        assert_eq!(unsafe { drachm_str_eq(&lit(E_ACUTE), &lit(E_COMBINING)) }, 0);
    }

    #[test]
    fn reverse_by_character() {
        assert_eq!(run(|o| unsafe { drachm_str_reverse(o, &lit("abc")) }), "cba");
        assert_eq!(run(|o| unsafe { drachm_str_reverse(o, &EMPTY) }), "");
        let s = format!("a{ACUTE}{FAMILY}{FLAG}z");
        let want = format!("z{FLAG}{FAMILY}a{ACUTE}");
        assert_eq!(run(|o| unsafe { drachm_str_reverse(o, &lit(&s)) }), want);
    }

    #[test]
    fn sub_ranges() {
        assert_eq!(sub("hello", 1, 3), "ell");
        assert_eq!(sub("hello", 0, 0), "h");
        assert_eq!(sub("hello", 0, -1), "hello");
        assert_eq!(sub("hello", -3, -2), "ll");
        assert_eq!(sub("hello", -5, 4), "hello");
        assert_eq!(sub("hello", 4, 4), "o");
        let s = format!("a{ACUTE}{FAMILY}{FLAG}z");
        assert_eq!(sub(&s, 1, 1), FAMILY);
        assert_eq!(sub(&s, 1, 2), format!("{FAMILY}{FLAG}"));
        assert_eq!(sub(&s, 0, 0), format!("a{ACUTE}"));
        assert_eq!(sub(&s, -1, -1), "z");
        assert_eq!(sub("x\r\ny", 1, 1), "\r\n");
    }

    #[test]
    #[should_panic(expected = "string index out of range")]
    fn sub_start_after_end() {
        sub("hello", 3, 2);
    }

    #[test]
    #[should_panic(expected = "string index out of range")]
    fn sub_end_too_big() {
        sub("hello", 0, 5);
    }

    #[test]
    #[should_panic(expected = "string index out of range")]
    fn sub_negative_too_far() {
        sub("hello", -6, 2);
    }

    #[test]
    #[should_panic(expected = "string index out of range")]
    fn sub_of_empty() {
        sub("", 0, 0);
    }

    #[test]
    #[should_panic(expected = "string index out of range")]
    fn sub_extreme_index() {
        sub("hello", i64::MIN, 2);
    }

    #[test]
    fn case_mapping() {
        let up = |s: &str| run(|o| unsafe { drachm_str_upper(o, &lit(s)) });
        let low = |s: &str| run(|o| unsafe { drachm_str_lower(o, &lit(s)) });
        assert_eq!(up("Hello, world 1"), "HELLO, WORLD 1");
        assert_eq!(low("Hello, WORLD 1"), "hello, world 1");
        assert_eq!(up("stra\u{df}e"), "STRASSE"); // full mapping changes the length
        assert_eq!(up(E_ACUTE), "\u{c9}");
        assert_eq!(low("\u{c9}"), E_ACUTE);
        assert_eq!(up(E_COMBINING), "E\u{301}");
        assert_eq!(up("\u{3c3}"), "\u{3a3}");
        assert_eq!(low("\u{3a3}"), "\u{3c3}"); // a lone capital sigma is not word-final
        assert_eq!(up(FAMILY), FAMILY);
        assert_eq!(low(""), "");
    }

    #[test]
    fn trimming() {
        let trim = |s: &str| run(|o| unsafe { drachm_str_trim(o, &lit(s)) });
        assert_eq!(trim("  hi \t\n"), "hi");
        assert_eq!(trim("a b"), "a b");
        assert_eq!(trim("   "), "");
        assert_eq!(trim(""), "");
        assert_eq!(trim("\u{a0}\u{2003}x\u{3000}"), "x"); // no-break, em and ideographic spaces
        assert_eq!(trim(&format!(" {FAMILY} ")), FAMILY);
    }

    #[test]
    fn find_and_friends() {
        assert_eq!(find("hello", "l"), 2);
        assert_eq!(find("hello", "lo"), 3);
        assert_eq!(find("hello", "hello"), 0);
        assert_eq!(find("hello", "z"), -1);
        assert_eq!(find("hello", "hellos"), -1);
        assert_eq!(find("hello", ""), 0);
        assert_eq!(find("", ""), 0);
        assert_eq!(find("", "a"), -1);
        // Character indexes, not byte indexes.
        let s = format!("{FAMILY}{E_COMBINING}x{FLAG}y");
        assert_eq!(find(&s, "x"), 2);
        assert_eq!(find(&s, "y"), 4);
        assert_eq!(find(&s, FLAG), 3);
        assert_eq!(find(&s, FAMILY), 0);
        // A match that starts inside a character belongs to that character.
        assert_eq!(find(&format!("a{ACUTE}b"), "\u{301}"), 0);
        assert_eq!(find("a\r\nb", "\nb"), 1);
        assert_eq!(find("a\rb\n", "\n"), 3);

        let (s, t) = (lit("hello"), lit("ell"));
        unsafe {
            assert_eq!(drachm_str_contains(&s, &t), 1);
            assert_eq!(drachm_str_contains(&s, &lit("")), 1);
            assert_eq!(drachm_str_contains(&s, &lit("lle")), 0);
            assert_eq!(drachm_str_starts_with(&s, &lit("he")), 1);
            assert_eq!(drachm_str_starts_with(&s, &t), 0);
            assert_eq!(drachm_str_starts_with(&s, &lit("")), 1);
            assert_eq!(drachm_str_starts_with(&lit("h"), &s), 0);
            assert_eq!(drachm_str_ends_with(&s, &lit("lo")), 1);
            assert_eq!(drachm_str_ends_with(&s, &t), 0);
            assert_eq!(drachm_str_ends_with(&s, &EMPTY), 1);
            assert_eq!(drachm_str_ends_with(&EMPTY, &s), 0);
        }
    }

    #[test]
    fn counting() {
        let count = |s: &str, t: &str| unsafe { drachm_str_count(&lit(s), &lit(t)) };
        assert_eq!(count("banana", "a"), 3);
        assert_eq!(count("banana", "an"), 2);
        assert_eq!(count("aaaa", "aa"), 2); // non-overlapping
        assert_eq!(count("banana", "x"), 0);
        assert_eq!(count("banana", ""), 0);
        assert_eq!(count("", ""), 0);
        assert_eq!(count(&format!("{FLAG}a{FLAG}"), FLAG), 2);
    }

    #[test]
    fn replacing() {
        let rep = |s: &str, f: &str, t: &str| run(|o| unsafe { drachm_str_replace(o, &lit(s), &lit(f), &lit(t)) });
        let lim = |s: &str, f: &str, t: &str, n| run(|o| unsafe { drachm_str_replace_limit(o, &lit(s), &lit(f), &lit(t), n) });
        assert_eq!(rep("banana", "an", "AN"), "bANANa");
        assert_eq!(rep("banana", "a", ""), "bnn");
        assert_eq!(rep("banana", "x", "y"), "banana");
        assert_eq!(rep("banana", "", "y"), "banana");
        assert_eq!(rep("", "a", "b"), "");
        assert_eq!(rep("aaa", "aa", "b"), "ba");
        assert_eq!(rep(&format!("a{FLAG}b"), FLAG, "-"), "a-b");
        assert_eq!(lim("banana", "a", "o", 2), "bonona");
        assert_eq!(lim("banana", "a", "o", 0), "banana");
        assert_eq!(lim("banana", "a", "o", 10), "bonono");
        assert_eq!(lim("banana", "a", "o", u64::MAX), "bonono");
    }

    #[test]
    fn inserting() {
        assert_eq!(insert("hello", 0, "X"), "Xhello");
        assert_eq!(insert("hello", 2, "X"), "heXllo");
        assert_eq!(insert("hello", 5, "X"), "helloX"); // index == len appends
        assert_eq!(insert("hello", -1, "X"), "hellXo");
        assert_eq!(insert("hello", -5, "X"), "Xhello");
        assert_eq!(insert("", 0, "X"), "X");
        assert_eq!(insert("hello", 1, ""), "hello");
        let s = format!("a{FAMILY}b");
        assert_eq!(insert(&s, 2, "-"), format!("a{FAMILY}-b"));
        assert_eq!(insert(&s, 1, "-"), format!("a-{FAMILY}b"));
        let s = format!("{E_COMBINING}{E_ACUTE}");
        assert_eq!(insert(&s, 1, "-"), format!("{E_COMBINING}-{E_ACUTE}"));
    }

    #[test]
    #[should_panic(expected = "string index out of range")]
    fn insert_past_end() {
        insert("hello", 6, "X");
    }

    #[test]
    #[should_panic(expected = "string index out of range")]
    fn insert_negative_too_far() {
        insert("hello", -6, "X");
    }

    #[test]
    fn removing() {
        assert_eq!(remove("hello", 1, 3), "ho");
        assert_eq!(remove("hello", 0, 0), "ello");
        assert_eq!(remove("hello", 0, -1), "");
        assert_eq!(remove("hello", -2, -1), "hel");
        assert_eq!(remove("hello", 4, 4), "hell");
        let s = format!("a{ACUTE}{FAMILY}{FLAG}z");
        assert_eq!(remove(&s, 1, 2), format!("a{ACUTE}z"));
        assert_eq!(remove(&s, 0, 0), format!("{FAMILY}{FLAG}z"));
    }

    #[test]
    #[should_panic(expected = "string index out of range")]
    fn remove_start_after_end() {
        remove("hello", 3, 1);
    }

    #[test]
    #[should_panic(expected = "string index out of range")]
    fn remove_out_of_range() {
        remove("hello", 0, 5);
    }

    #[test]
    fn repeating() {
        let rep = |s: &str, n| run(|o| unsafe { drachm_str_repeat(o, &lit(s), n) });
        assert_eq!(rep("ab", 3), "ababab");
        assert_eq!(rep("ab", 1), "ab");
        assert_eq!(rep("ab", 0), "");
        assert_eq!(rep("", 1000), "");
        assert_eq!(rep(FLAG, 2), format!("{FLAG}{FLAG}"));
    }

    #[test]
    #[should_panic(expected = "string too long")]
    fn repeat_overflow() {
        run(|o| unsafe { drachm_str_repeat(o, &lit("ab"), u64::MAX) });
    }

    #[test]
    fn padding_text() {
        assert_eq!(pad_left("42", 5, "0"), "00042");
        assert_eq!(pad_right("42", 5, "."), "42...");
        assert_eq!(pad_left("42", 2, "0"), "42");
        assert_eq!(pad_left("42", 1, "0"), "42");
        assert_eq!(pad_left("", 3, "ab"), "aba"); // fill repeated and cut to fit
        assert_eq!(pad_right("x", 6, "ab"), "xababa");
        assert_eq!(pad_left("", 0, "-"), "");
        // Width counts characters on both the text and the fill.
        assert_eq!(pad_left(E_COMBINING, 3, "*"), format!("**{E_COMBINING}"));
        assert_eq!(pad_right(FAMILY, 3, "-"), format!("{FAMILY}--"));
        assert_eq!(pad_left("x", 4, &format!("{FLAG}{FAMILY}")), format!("{FLAG}{FAMILY}{FLAG}x"));
        assert_eq!(pad_right("x", 2, &format!("{FLAG}{FAMILY}")), format!("x{FLAG}"));
    }

    #[test]
    #[should_panic(expected = "empty padding fill")]
    fn pad_empty_fill() {
        pad_left("x", 3, "");
    }

    #[test]
    fn radix_text() {
        assert_eq!(radix(255, false, 16, false), "ff");
        assert_eq!(radix(255, false, 16, true), "FF");
        assert_eq!(radix(5, false, 2, false), "101");
        assert_eq!(radix(8, false, 8, false), "10");
        assert_eq!(radix(0, false, 2, false), "0");
        assert_eq!(radix(-255, true, 16, false), "-ff");
        assert_eq!(radix(-1, true, 2, false), "-1");
        assert_eq!(radix(i128::MIN, true, 16, false), format!("-8{}", "0".repeat(31)));
        assert_eq!(radix(i128::MIN, true, 2, false).len(), 129);
        // The same bits unsigned are a large positive value.
        assert_eq!(radix(-1, false, 16, true), "F".repeat(32));
        assert_eq!(radix(i64::MIN as i128, true, 16, false), "-8000000000000000");
        assert_eq!(radix(35, false, 36, true), "Z");
    }

    #[test]
    #[should_panic(expected = "invalid number base")]
    fn radix_bad_base() {
        radix(1, false, 1, false);
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn integer_boundaries() {
        let ok = |t: &str, signed, bits| parse_int(t, signed, bits);
        assert_eq!(ok("255", false, 8), Some((255, 0)));
        assert_eq!(ok("256", false, 8), None);
        assert_eq!(ok("127", true, 8), Some((127, 0)));
        assert_eq!(ok("128", true, 8), None);
        assert_eq!(ok("-128", true, 8), Some((0xffff_ffff_ffff_ff80, u64::MAX)));
        assert_eq!(ok("-129", true, 8), None);
        assert_eq!(ok("18446744073709551615", false, 64), Some((u64::MAX, 0)));
        assert_eq!(ok("18446744073709551616", false, 64), None);
        assert_eq!(ok("-9223372036854775808", true, 64), Some((1 << 63, u64::MAX)));
        assert_eq!(ok("-9223372036854775809", true, 64), None);
        assert_eq!(ok("340282366920938463463374607431768211455", false, 128), Some((u64::MAX, u64::MAX)));
        assert_eq!(ok("340282366920938463463374607431768211456", false, 128), None);
        assert_eq!(ok("-170141183460469231731687303715884105728", true, 128), Some((0, 1 << 63)));
        assert_eq!(ok("170141183460469231731687303715884105728", true, 128), None);
        assert_eq!(ok("-0", false, 32), None);
        assert_eq!(ok("-0", true, 32), Some((0, 0)));
    }

    #[test]
    fn integer_grammar() {
        for bad in ["", " ", "-", "+", "+-1", "-+1", "- 1", "1 2", "1e3", "0x10", "1_000", "3.0", "--1", "١", "1,000"] {
            assert_eq!(parse_int(bad, true, 64), None, "{bad:?} as an integer");
        }
        assert_eq!(parse_int("007", false, 8), Some((7, 0)));
        assert_eq!(parse_int("-1", false, 64), None);
        assert_eq!(parse_int("-0", false, 8), None);
    }

    #[test]
    fn integer_whitespace_and_plus() {
        assert_eq!(parse_int(" 42 ", false, 8), Some((42, 0)));
        assert_eq!(parse_int("\t42\n", false, 8), Some((42, 0)));
        assert_eq!(parse_int("+42", false, 8), Some((42, 0)));
        assert_eq!(parse_int(" +42 ", true, 8), Some((42, 0)));
        assert_eq!(parse_int(" -128 ", true, 8), Some((0xffff_ffff_ffff_ff80, u64::MAX)));
        assert_eq!(parse_int("+256", false, 8), None);
        assert_eq!(parse_int(" 256 ", false, 8), None);
        assert_eq!(parse_int(" 0 ", false, 8), Some((0, 0)));
    }

    #[test]
    fn float_grammar() {
        for good in ["0", "-0", "3.25", "1e3", "1E-3", "1e+3", "-2.5e10", "+1", " 1 ", "\n-2.5\t", ".5", "5.", "-.5", "+5.", ".5e2", "5.e2"] {
            assert!(is_float_text(good), "{good:?}");
        }
        for bad in ["", " ", "-", "+", ".", "-.", "+.", "1e", "1e+", ".e3", "inf", "nan", "infinity", "0x1p3", "1 2", "1_0", "+-1", "1.2.3"] {
            assert!(!is_float_text(bad), "{bad:?}");
        }
    }

    #[test]
    fn float_overflow_is_bad_text() {
        let f = |t: &str| {
            let s = DrachmStr { ptr: t.as_ptr() as *mut u8, len: t.len() as u64, cap: 0 };
            let mut v = 0f64;
            let ok64 = unsafe { drachm_str_parse_f64(&s, &mut v) };
            let mut w = 0f32;
            let ok32 = unsafe { drachm_str_parse_f32(&s, &mut w) };
            (ok64, ok32)
        };
        assert_eq!(f("1e308"), (1, 0));
        assert_eq!(f("1e39"), (1, 0));
        assert_eq!(f("1e400"), (0, 0));
        assert_eq!(f("1e-400"), (1, 1));
    }

    #[test]
    fn from_bytes_replaces_bad_utf8() {
        let raw = b"ok \xff\xfe \xc3\xa9";
        let mut out = DrachmStr { ptr: ptr::null_mut(), len: 0, cap: 0 };
        unsafe { drachm_str_from_bytes(&mut out, raw.as_ptr(), raw.len() as u64) };
        assert_eq!(unsafe { text(&out) }, "ok \u{fffd}\u{fffd} \u{e9}");
        unsafe { drachm_str_drop(&mut out) };
        let mut empty = DrachmStr { ptr: ptr::null_mut(), len: 1, cap: 1 };
        unsafe { drachm_str_from_bytes(&mut empty, ptr::null(), 0) };
        assert_eq!((empty.len, empty.cap), (0, 0));
    }
}
