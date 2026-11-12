//! `split`, `lines` and `chars` of a string (S6C.2): each gives a new list of
//! owned strings, the list in the `drachm_list` shape of runtime/lists.h.
//! Every piece is a fresh copy, so the list owns it like any `string[?]`.

use unicode_segmentation::UnicodeSegmentation;

use crate::lists::{DrachmList, EMPTY_LIST, drachm_list_push};
use crate::{DrachmStr, EMPTY, bytes, emit};

/// Size and alignment of a `string` item in a list.
const STR_SIZE: u64 = std::mem::size_of::<DrachmStr>() as u64;
const STR_ALIGN: u64 = std::mem::align_of::<DrachmStr>() as u64;

/// Appends one owned copy of `data` to `out`.
unsafe fn push_copy(out: *mut DrachmList, data: &[u8]) {
    let mut piece = EMPTY;
    unsafe { emit(&mut piece, data) };
    unsafe { drachm_list_push(out, &piece as *const DrachmStr as *const u8, STR_SIZE, STR_ALIGN) };
}

/// The first place `pat` occurs in `text`, if it does.
fn find(text: &[u8], pat: &[u8]) -> Option<usize> {
    text.windows(pat.len()).position(|w| w == pat)
}

/// The pieces of `s` between the occurrences of `sep`, in order. Empty pieces
/// are kept: `"a,,b"` gives `a`, ``, `b`, and `""` gives one empty piece.
/// An empty separator gives the same grapheme clusters as `chars`.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_split(out: *mut DrachmList, s: *const DrachmStr, sep: *const DrachmStr) {
    let (text, sep) = unsafe { (bytes(s), bytes(sep)) };
    if sep.is_empty() {
        unsafe { drachm_str_chars(out, s) };
        return;
    }
    unsafe { out.write(EMPTY_LIST) };
    let mut rest = text;
    while let Some(at) = find(rest, sep) {
        unsafe { push_copy(out, &rest[..at]) };
        rest = &rest[at + sep.len()..];
    }
    unsafe { push_copy(out, rest) };
}

/// The lines of `s`, without their line ends (`\n`, or `\r\n`). A last line
/// that ends without a line end is a line too, and an empty `s` has no lines.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_lines(out: *mut DrachmList, s: *const DrachmStr) {
    let text = unsafe { bytes(s) };
    unsafe { out.write(EMPTY_LIST) };
    let mut rest = text;
    while !rest.is_empty() {
        match rest.iter().position(|&b| b == b'\n') {
            Some(at) => {
                let line = &rest[..at];
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                unsafe { push_copy(out, line) };
                rest = &rest[at + 1..];
            }
            None => {
                unsafe { push_copy(out, rest) };
                break;
            }
        }
    }
}

/// The characters (grapheme clusters, as `len` counts them) of `s`, one
/// owned string each. An empty `s` gives an empty list.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_chars(out: *mut DrachmList, s: *const DrachmStr) {
    let text = unsafe { std::str::from_utf8_unchecked(bytes(s)) };
    unsafe { out.write(EMPTY_LIST) };
    for g in text.graphemes(true) {
        unsafe { push_copy(out, g.as_bytes()) };
    }
}
