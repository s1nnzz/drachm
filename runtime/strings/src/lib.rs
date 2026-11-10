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
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

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
    EXIT_HOOK.call_once(|| unsafe {
        libc::atexit(report_leaks);
    });
    let p = unsafe { libc::malloc(cap as usize) } as *mut u8;
    if p.is_null() {
        panic_with(c"out of memory");
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

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_concat(out: *mut DrachmStr, a: *const DrachmStr, b: *const DrachmStr) {
    let (a, b) = unsafe { (bytes(a), bytes(b)) };
    let mut v = Vec::with_capacity(a.len() + b.len());
    v.extend_from_slice(a);
    v.extend_from_slice(b);
    unsafe { emit(out, &v) };
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

/// Byte offsets where each character starts, followed by the string's length,
/// so character `i` spans `offsets[i]..offsets[i + 1]`.
fn char_offsets(s: &str) -> Vec<usize> {
    let mut v: Vec<usize> = s.grapheme_indices(true).map(|(i, _)| i).collect();
    v.push(s.len());
    v
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
    unsafe { text(s).graphemes(true).count() as u64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_is_empty(s: *const DrachmStr) -> u8 {
    unsafe { ((*s).len == 0) as u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_reverse(out: *mut DrachmStr, s: *const DrachmStr) {
    let mut v = Vec::with_capacity(unsafe { (*s).len } as usize);
    for g in unsafe { text(s) }.graphemes(true).rev() {
        v.extend_from_slice(g.as_bytes());
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
    let offs = char_offsets(t);
    let at = offs[resolve(index, offs.len() - 1, true)];
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
    let mut v = Vec::with_capacity(total as usize);
    for _ in 0..n {
        v.extend_from_slice(b);
    }
    unsafe { emit(out, &v) };
}

/// Shared by padLeft and padRight: the padding text that brings `t` up to `width`
/// characters, or `None` when it is already wide enough.
fn padding(t: &str, width: u64, fill: &str) -> Option<String> {
    if fill.is_empty() {
        panic_with(c"empty padding fill");
    }
    let have = t.graphemes(true).count() as u64;
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
