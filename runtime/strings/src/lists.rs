//! Lists: `{ptr, len, cap}` buffers of raw elements, the C side of
//! runtime/lists.h. Elements are moved and copied as bytes; the caller gives
//! each element's size and alignment. Every buffer comes from the counted
//! allocator in lib.rs, so the leak check sees it.
//!
//! Zero-sized elements never allocate: `ptr` and `cap` stay 0 and `len` counts.
//! Callbacks (drop, copy, eq) still run once per element, with a dangling
//! pointer, so a counting drop sees every element.

use std::ffi::CString;
use std::ptr;

use crate::{DrachmStr, bytes, counted_free, counted_realloc, drachm_str_alloc, emit_parts, panic_with};

/// The C callback types of runtime/lists.h. NULL is `None`.
pub(crate) type DropFn = Option<unsafe extern "C" fn(*mut u8)>;
pub(crate) type CopyFn = Option<unsafe extern "C" fn(*mut u8, *const u8)>;
pub(crate) type EqFn = Option<unsafe extern "C" fn(*const u8, *const u8) -> u8>;

/// Same layout as `drachm_list` in runtime/lists.h (and as a string).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DrachmList {
    pub ptr: *mut u8,
    pub len: u64,
    pub cap: u64,
}

pub(crate) const EMPTY_LIST: DrachmList = DrachmList { ptr: ptr::null_mut(), len: 0, cap: 0 };

const MIN_CAP: u64 = 4;
/// Largest byte count a Rust allocation may have; buffers stay under it.
const MAX_BYTES: u64 = isize::MAX as u64;

/// Panics with a message built at run time, through the one C panic path.
pub(crate) fn panic_text(msg: &str) -> ! {
    let c = CString::new(msg).unwrap_or_default();
    unsafe { crate::drachm_panic(c.as_ptr()) }
}

fn index_panic(i: i64, len: u64) -> ! {
    panic_text(&format!("index {i} is outside the list (length {len})"))
}

/// Element and entry alignments are powers of 2 up to 16 (malloc gives 16).
pub(crate) fn check_align(align: u64) {
    if align == 0 || !align.is_power_of_two() || align > 16 {
        panic_with(c"bad element alignment");
    }
}

/// A non-NULL, suitably aligned address for zero-sized elements.
fn dangling() -> *mut u8 {
    std::ptr::NonNull::<u8>::dangling().as_ptr()
}

/// The address of element `i` in a buffer `data`.
unsafe fn slot(data: *mut u8, i: u64, size: u64) -> *mut u8 {
    if size == 0 { dangling() } else { unsafe { data.add((i * size) as usize) } }
}

/// The position `i` names in a list of `len` elements (negative counts from
/// the end), or None when there is no such position.
pub(crate) fn resolve(len: u64, i: i64) -> Option<u64> {
    if i >= 0 {
        let u = i as u64;
        (u < len).then_some(u)
    } else {
        let back = i.unsigned_abs();
        (back <= len).then(|| len - back)
    }
}

/// Like `resolve`, but for inserting: `len` itself (the end) is allowed.
fn resolve_insert(len: u64, i: i64) -> Option<u64> {
    if i >= 0 {
        let u = i as u64;
        (u <= len).then_some(u)
    } else {
        let back = i.unsigned_abs();
        (back <= len).then(|| len - back)
    }
}

fn index_or_panic(len: u64, i: i64) -> u64 {
    resolve(len, i).unwrap_or_else(|| index_panic(i, len))
}

/// Makes room for one more element. Zero-sized elements never allocate.
unsafe fn make_room(l: *mut DrachmList, size: u64) {
    let (len, cap) = unsafe { ((*l).len, (*l).cap) };
    if size == 0 || len < cap {
        return;
    }
    let new_cap = (cap * 2).max(MIN_CAP);
    let bytes = new_cap.checked_mul(size).filter(|&b| b <= MAX_BYTES).unwrap_or_else(|| panic_with(c"list is too long"));
    unsafe {
        (*l).ptr = counted_realloc((*l).ptr, bytes);
        (*l).cap = new_cap;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_index_panic(i: i64, len: u64) -> ! {
    index_panic(i, len)
}

#[unsafe(no_mangle)]
pub extern "C-unwind" fn drachm_list_index(len: u64, i: i64) -> u64 {
    index_or_panic(len, i)
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_new(out: *mut DrachmList) {
    unsafe { out.write(EMPTY_LIST) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_with_cap(out: *mut DrachmList, cap: u64, size: u64, align: u64) {
    check_align(align);
    if size == 0 || cap == 0 {
        unsafe { out.write(EMPTY_LIST) };
        return;
    }
    let bytes = cap.checked_mul(size).filter(|&b| b <= MAX_BYTES).unwrap_or_else(|| panic_with(c"list is too long"));
    let p = drachm_str_alloc(bytes);
    unsafe { out.write(DrachmList { ptr: p, len: 0, cap }) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_copy(
    out: *mut DrachmList,
    l: *const DrachmList,
    size: u64,
    align: u64,
    copy_elem: CopyFn,
) {
    check_align(align);
    let (src, n) = unsafe { ((*l).ptr, (*l).len) };
    if n == 0 {
        unsafe { out.write(EMPTY_LIST) };
        return;
    }
    let bytes = n.checked_mul(size).filter(|&b| b <= MAX_BYTES).unwrap_or_else(|| panic_with(c"list is too long"));
    let dst = if size == 0 { ptr::null_mut() } else { drachm_str_alloc(bytes) };
    for i in 0..n {
        let (s, d) = unsafe { (slot(src, i, size), slot(dst, i, size)) };
        match copy_elem {
            Some(f) => unsafe { f(d, s) },
            None => unsafe { ptr::copy_nonoverlapping(s, d, size as usize) },
        }
    }
    let cap = if size == 0 { 0 } else { n };
    unsafe { out.write(DrachmList { ptr: dst, len: n, cap }) };
}

/// `elem` must not point into the list's own buffer (growing may free it).
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_push(l: *mut DrachmList, elem: *const u8, size: u64, align: u64) {
    check_align(align);
    unsafe {
        make_room(l, size);
        let len = (*l).len;
        if size > 0 {
            ptr::copy_nonoverlapping(elem, slot((*l).ptr, len, size), size as usize);
        }
        (*l).len = len + 1;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_at(l: *const DrachmList, i: i64, size: u64) -> *mut u8 {
    let (data, len) = unsafe { ((*l).ptr, (*l).len) };
    let idx = index_or_panic(len, i);
    unsafe { slot(data, idx, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_get(l: *const DrachmList, i: i64, size: u64) -> *mut u8 {
    let (data, len) = unsafe { ((*l).ptr, (*l).len) };
    match resolve(len, i) {
        Some(idx) => unsafe { slot(data, idx, size) },
        None => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_pop(l: *mut DrachmList, out: *mut u8, size: u64) -> u8 {
    let (data, len) = unsafe { ((*l).ptr, (*l).len) };
    if len == 0 {
        return 0;
    }
    let last = len - 1;
    unsafe {
        (*l).len = last;
        if size > 0 {
            ptr::copy_nonoverlapping(slot(data, last, size), out, size as usize);
        }
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_insert(l: *mut DrachmList, i: i64, elem: *const u8, size: u64, align: u64) {
    check_align(align);
    let len = unsafe { (*l).len };
    let at = resolve_insert(len, i).unwrap_or_else(|| index_panic(i, len));
    unsafe {
        make_room(l, size);
        if size > 0 {
            let data = (*l).ptr;
            let moved = len - at;
            if moved > 0 {
                ptr::copy(slot(data, at, size), slot(data, at + 1, size), (moved * size) as usize);
            }
            ptr::copy_nonoverlapping(elem, slot(data, at, size), size as usize);
        }
        (*l).len = len + 1;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_remove(l: *mut DrachmList, i: i64, out: *mut u8, size: u64) {
    let (data, len) = unsafe { ((*l).ptr, (*l).len) };
    let idx = index_or_panic(len, i);
    unsafe {
        if size > 0 {
            ptr::copy_nonoverlapping(slot(data, idx, size), out, size as usize);
            let after = len - idx - 1;
            if after > 0 {
                ptr::copy(slot(data, idx + 1, size), slot(data, idx, size), (after * size) as usize);
            }
        }
        (*l).len = len - 1;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_swap_remove(l: *mut DrachmList, i: i64, out: *mut u8, size: u64) {
    let (data, len) = unsafe { ((*l).ptr, (*l).len) };
    let idx = index_or_panic(len, i);
    let last = len - 1;
    unsafe {
        if size > 0 {
            ptr::copy_nonoverlapping(slot(data, idx, size), out, size as usize);
            if idx != last {
                ptr::copy_nonoverlapping(slot(data, last, size), slot(data, idx, size), size as usize);
            }
        }
        (*l).len = last;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_index_of(l: *const DrachmList, v: *const u8, size: u64, eq: EqFn) -> i64 {
    let (data, len) = unsafe { ((*l).ptr, (*l).len) };
    let Some(eq) = eq else { panic_with(c"list element has no equality") };
    for i in 0..len {
        if unsafe { eq(slot(data, i, size), v) } != 0 {
            return i as i64;
        }
    }
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_clear(l: *mut DrachmList, size: u64, drop_elem: DropFn) {
    let (data, len) = unsafe { ((*l).ptr, (*l).len) };
    // Empty first: the elements' drop code must not see them as still there.
    unsafe { (*l).len = 0 };
    if let Some(f) = drop_elem {
        for i in 0..len {
            unsafe { f(slot(data, i, size)) };
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_drop(l: *mut DrachmList, size: u64, drop_elem: DropFn) {
    let (data, len, cap) = unsafe { ((*l).ptr, (*l).len, (*l).cap) };
    unsafe { l.write(EMPTY_LIST) };
    if let Some(f) = drop_elem {
        for i in 0..len {
            unsafe { f(slot(data, i, size)) };
        }
    }
    if cap > 0 {
        counted_free(data);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_list_join(out: *mut DrachmStr, l: *const DrachmList, sep: *const DrachmStr) {
    let (data, len) = unsafe { ((*l).ptr, (*l).len) };
    let sep = unsafe { bytes(sep) };
    let mut parts: Vec<&[u8]> = Vec::with_capacity(2 * len as usize);
    for i in 0..len {
        parts.push(unsafe { bytes(slot(data, i, 24) as *const DrachmStr) });
        if i + 1 < len {
            parts.push(sep);
        }
    }
    unsafe { emit_parts(out, &parts) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_live;
    use std::cell::{Cell, RefCell};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    thread_local! {
        static DROPS: Cell<u64> = const { Cell::new(0) };
        static DROPPED_FIRST: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        static COPIES: Cell<u64> = const { Cell::new(0) };
    }

    /// Counts a drop without reading the element (safe for zero-sized ones).
    unsafe extern "C" fn count_drop(_: *mut u8) {
        DROPS.with(|d| d.set(d.get() + 1));
    }

    /// Counts a drop and records the element's first byte.
    unsafe extern "C" fn record_drop(p: *mut u8) {
        DROPS.with(|d| d.set(d.get() + 1));
        let first = unsafe { *p };
        DROPPED_FIRST.with(|v| v.borrow_mut().push(first));
    }

    /// A copy callback for 8-byte elements that counts its calls.
    unsafe extern "C" fn copy8(dst: *mut u8, src: *const u8) {
        COPIES.with(|c| c.set(c.get() + 1));
        unsafe { ptr::copy_nonoverlapping(src, dst, 8) };
    }

    unsafe extern "C" fn eq_bytes(a: *const u8, b: *const u8) -> u8 {
        // Elements in these tests are 3 bytes, compared as bytes.
        (unsafe { std::slice::from_raw_parts(a, 3) } == unsafe { std::slice::from_raw_parts(b, 3) }) as u8
    }

    fn drops() -> u64 {
        DROPS.with(|d| d.get())
    }

    /// An element of `size` bytes holding `seed` in its first byte and a
    /// pattern after it.
    fn elem(size: u64, seed: u64) -> Vec<u8> {
        (0..size).map(|k| (seed as u8).wrapping_add(k as u8).wrapping_mul(3)).collect()
    }

    fn contents(l: &DrachmList, size: u64) -> Vec<Vec<u8>> {
        (0..l.len)
            .map(|i| unsafe {
                let p = drachm_list_at(l, i as i64, size);
                if size == 0 { Vec::new() } else { std::slice::from_raw_parts(p, size as usize).to_vec() }
            })
            .collect()
    }

    fn rng(st: &mut u64) -> u64 {
        *st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *st >> 33
    }

    unsafe fn push(l: &mut DrachmList, e: &[u8], align: u64) {
        unsafe { drachm_list_push(l, e.as_ptr(), e.len() as u64, align) };
    }

    /// (size, align) pairs the tests run every operation over.
    const SHAPES: &[(u64, u64)] = &[(0, 1), (1, 1), (3, 1), (8, 8), (24, 8), (40, 8), (16, 16)];

    #[test]
    fn empty_header_is_usable() {
        let mut l = DrachmList { ptr: ptr::null_mut(), len: 0, cap: 0 };
        let mut out = [0u8; 3];
        unsafe {
            assert_eq!(drachm_list_pop(&mut l, out.as_mut_ptr(), 3), 0);
            assert!(drachm_list_get(&l, 0, 3).is_null());
            push(&mut l, &[1, 2, 3], 1);
            assert_eq!(l.len, 1);
            assert_eq!(drachm_list_pop(&mut l, out.as_mut_ptr(), 3), 1);
            assert_eq!(out, [1, 2, 3]);
            drachm_list_drop(&mut l, 3, None);
        }
        assert!(l.ptr.is_null() && l.len == 0 && l.cap == 0);
    }

    #[test]
    fn push_and_pop_every_shape() {
        let base = thread_live();
        for &(size, align) in SHAPES {
            let mut l = EMPTY_LIST;
            let items: Vec<Vec<u8>> = (0..1000).map(|i| elem(size, i)).collect();
            for e in &items {
                unsafe { push(&mut l, e, align) };
            }
            assert_eq!(l.len, 1000);
            assert_eq!(contents(&l, size), items, "size {size}");
            if size > 0 {
                let p = unsafe { drachm_list_at(&l, 999, size) };
                assert_eq!(p as usize % align as usize, 0, "alignment of size {size}");
            }
            let mut out = vec![0u8; size as usize];
            for e in items.iter().rev() {
                assert_eq!(unsafe { drachm_list_pop(&mut l, out.as_mut_ptr(), size) }, 1);
                assert_eq!(&out, e);
            }
            assert_eq!(unsafe { drachm_list_pop(&mut l, out.as_mut_ptr(), size) }, 0);
            unsafe { drachm_list_drop(&mut l, size, None) };
        }
        assert_eq!(thread_live(), base, "every buffer freed");
    }

    #[test]
    fn growth_over_many_reallocations() {
        let base = thread_live();
        let mut l = EMPTY_LIST;
        for i in 0..100_000u64 {
            unsafe { push(&mut l, &i.to_le_bytes(), 8) };
        }
        assert_eq!(l.len, 100_000);
        assert!(l.cap >= l.len && l.cap.is_power_of_two());
        for i in (0..100_000u64).step_by(997) {
            let p = unsafe { drachm_list_at(&l, i as i64, 8) };
            assert_eq!(unsafe { *(p as *const u64) }, i);
        }
        unsafe { drachm_list_drop(&mut l, 8, None) };
        assert_eq!(thread_live(), base);
    }

    /// Random inserts, removes, pops and reads, checked against a Vec model.
    #[test]
    fn model_check_insert_remove_swap() {
        let base = thread_live();
        for &(size, align) in SHAPES {
            let mut st = 0x9e37_79b9 ^ size;
            let mut l = EMPTY_LIST;
            let mut model: Vec<Vec<u8>> = Vec::new();
            for step in 0..3000u64 {
                let len = model.len() as u64;
                let r = rng(&mut st);
                let mut out = vec![0u8; size as usize];
                match r % 6 {
                    0 | 1 => {
                        let e = elem(size, step);
                        unsafe { push(&mut l, &e, align) };
                        model.push(e);
                    }
                    2 => {
                        // An index in 0..=len, given as a positive or a negative number
                        // (the end has no negative form: -0 is the front).
                        let at = rng(&mut st) % (len + 1);
                        let i = if r & 8 == 0 || at == len { at as i64 } else { -((len - at) as i64) };
                        let e = elem(size, step + 7);
                        unsafe { drachm_list_insert(&mut l, i, e.as_ptr(), size, align) };
                        model.insert(at as usize, e);
                    }
                    3 if len > 0 => {
                        let at = rng(&mut st) % len;
                        let i = if r & 8 == 0 { at as i64 } else { -((len - at) as i64) };
                        unsafe { drachm_list_remove(&mut l, i, out.as_mut_ptr(), size) };
                        assert_eq!(out, model.remove(at as usize), "remove size {size} step {step}");
                    }
                    4 if len > 0 => {
                        let at = rng(&mut st) % len;
                        unsafe { drachm_list_swap_remove(&mut l, at as i64, out.as_mut_ptr(), size) };
                        assert_eq!(out, model[at as usize], "swap_remove size {size} step {step}");
                        let last = model.pop().unwrap();
                        if at as usize != model.len() {
                            model[at as usize] = last;
                        }
                    }
                    5 if len > 0 => {
                        assert_eq!(unsafe { drachm_list_pop(&mut l, out.as_mut_ptr(), size) }, 1);
                        assert_eq!(out, model.pop().unwrap());
                    }
                    _ => {}
                }
                assert_eq!(l.len as usize, model.len());
                if step % 251 == 0 {
                    assert_eq!(contents(&l, size), model, "size {size} step {step}");
                }
            }
            assert_eq!(contents(&l, size), model, "size {size} at the end");
            unsafe { drachm_list_drop(&mut l, size, None) };
        }
        assert_eq!(thread_live(), base);
    }

    #[test]
    fn negative_index_counts_from_the_end() {
        let mut l = EMPTY_LIST;
        for b in [10u8, 20, 30] {
            unsafe { push(&mut l, &[b, 0, 0], 1) };
        }
        unsafe {
            assert_eq!(*drachm_list_at(&l, -1, 3), 30);
            assert_eq!(*drachm_list_at(&l, -3, 3), 10);
            assert!(drachm_list_get(&l, -4, 3).is_null());
            assert!(drachm_list_get(&l, 3, 3).is_null());
            assert_eq!(*drachm_list_get(&l, 2, 3), 30);
            assert_eq!(drachm_list_index(3, -2), 1);
            drachm_list_drop(&mut l, 3, None);
        }
    }

    #[test]
    fn index_out_of_range_panics_with_the_message() {
        let mut l = EMPTY_LIST;
        unsafe {
            push(&mut l, &[1, 2, 3], 1);
            push(&mut l, &[1, 2, 3], 1);
            push(&mut l, &[1, 2, 3], 1);
        }
        let msg = |r: Result<*mut u8, Box<dyn std::any::Any + Send>>| {
            let e = r.expect_err("expected a panic");
            e.downcast_ref::<String>().cloned().unwrap_or_default()
        };
        let m = msg(catch_unwind(AssertUnwindSafe(|| unsafe { drachm_list_at(&l, 5, 3) })));
        assert_eq!(m, "index 5 is outside the list (length 3)");
        let m = msg(catch_unwind(AssertUnwindSafe(|| unsafe { drachm_list_at(&l, -4, 3) })));
        assert_eq!(m, "index -4 is outside the list (length 3)");
        let m = msg(catch_unwind(AssertUnwindSafe(|| {
            let mut out = [0u8; 3];
            unsafe { drachm_list_remove(&mut { l }, 3, out.as_mut_ptr(), 3) };
            ptr::null_mut()
        })));
        assert_eq!(m, "index 3 is outside the list (length 3)");
        unsafe { drachm_list_drop(&mut l, 3, None) };
    }

    #[test]
    fn drop_runs_once_per_element_not_for_removed_ones() {
        let base = thread_live();
        let mut l = EMPTY_LIST;
        DROPS.with(|d| d.set(0));
        DROPPED_FIRST.with(|v| v.borrow_mut().clear());
        for b in 0..10u8 {
            unsafe { push(&mut l, &[b, 1, 2, 3, 4, 5, 6, 7], 8) };
        }
        let mut out = [0u8; 8];
        unsafe {
            drachm_list_remove(&mut l, 0, out.as_mut_ptr(), 8);
            assert_eq!(out[0], 0);
            drachm_list_pop(&mut l, out.as_mut_ptr(), 8);
            drachm_list_swap_remove(&mut l, 2, out.as_mut_ptr(), 8);
        }
        assert_eq!(drops(), 0, "removed items are the caller's to drop");
        unsafe { drachm_list_clear(&mut l, 8, Some(record_drop)) };
        assert_eq!(drops(), 7);
        assert_eq!(l.len, 0);
        assert!(l.cap >= 10, "clear keeps the buffer");
        DROPPED_FIRST.with(|v| assert_eq!(*v.borrow(), vec![1, 2, 8, 4, 5, 6, 7]));
        unsafe { push(&mut l, &[9; 8], 8) };
        unsafe { drachm_list_drop(&mut l, 8, Some(record_drop)) };
        assert_eq!(drops(), 8);
        assert_eq!(thread_live(), base);
    }

    #[test]
    fn zero_sized_elements_count_and_call_back() {
        let base = thread_live();
        let mut l = EMPTY_LIST;
        DROPS.with(|d| d.set(0));
        for _ in 0..5 {
            unsafe { drachm_list_push(&mut l, ptr::null(), 0, 1) };
        }
        assert_eq!(l.len, 5);
        assert!(l.ptr.is_null() && l.cap == 0, "nothing allocated");
        unsafe {
            assert!(!drachm_list_at(&l, 4, 0).is_null());
            drachm_list_insert(&mut l, 2, ptr::null(), 0, 1);
            drachm_list_remove(&mut l, 0, ptr::null_mut(), 0);
            assert_eq!(l.len, 5);
            drachm_list_clear(&mut l, 0, Some(count_drop));
            assert_eq!(drops(), 5);
            drachm_list_drop(&mut l, 0, None);
        }
        assert_eq!(thread_live(), base);
    }

    #[test]
    fn copy_is_independent_and_calls_the_copy_callback() {
        let base = thread_live();
        COPIES.with(|c| c.set(0));
        let mut src = EMPTY_LIST;
        for i in 0..50u64 {
            unsafe { push(&mut src, &i.to_le_bytes(), 8) };
        }
        let mut out = EMPTY_LIST;
        unsafe { drachm_list_copy(&mut out, &src, 8, 8, Some(copy8)) };
        assert_eq!(COPIES.with(|c| c.get()), 50);
        assert_eq!(out.len, 50);
        unsafe {
            let mut tmp = [0u8; 8];
            drachm_list_swap_remove(&mut src, 0, tmp.as_mut_ptr(), 8);
            drachm_list_drop(&mut src, 8, None);
            assert_eq!(*(drachm_list_at(&out, 0, 8) as *const u64), 0);
            assert_eq!(*(drachm_list_at(&out, 49, 8) as *const u64), 49);
            drachm_list_drop(&mut out, 8, None);
        }
        // NULL callback: a memcpy copy.
        let mut src = EMPTY_LIST;
        unsafe {
            push(&mut src, &[1, 2, 3], 1);
            let mut out = EMPTY_LIST;
            drachm_list_copy(&mut out, &src, 3, 1, None);
            assert_eq!(contents(&out, 3), vec![vec![1, 2, 3]]);
            drachm_list_drop(&mut out, 3, None);
            drachm_list_drop(&mut src, 3, None);
        }
        assert_eq!(thread_live(), base);
    }

    #[test]
    fn index_of_uses_the_equality_callback() {
        let mut l = EMPTY_LIST;
        unsafe {
            push(&mut l, b"abc", 1);
            push(&mut l, b"def", 1);
            push(&mut l, b"abc", 1);
            assert_eq!(drachm_list_index_of(&l, b"def".as_ptr(), 3, Some(eq_bytes)), 1);
            assert_eq!(drachm_list_index_of(&l, b"xyz".as_ptr(), 3, Some(eq_bytes)), -1);
            drachm_list_drop(&mut l, 3, None);
        }
    }

    #[test]
    fn with_cap_and_join() {
        let base = thread_live();
        let lit = |s: &'static str| DrachmStr { ptr: s.as_ptr() as *mut u8, len: s.len() as u64, cap: 0 };
        let mut l = EMPTY_LIST;
        unsafe {
            drachm_list_with_cap(&mut l, 10, 24, 8);
            assert_eq!((l.len, l.cap), (0, 10));
            let parts = [lit("ab"), lit(""), lit("cde")];
            for p in &parts {
                drachm_list_push(&mut l, p as *const DrachmStr as *const u8, 24, 8);
            }
            assert_eq!(l.cap, 10, "no growth needed");
            let sep = lit(", ");
            let mut out = DrachmStr { ptr: ptr::null_mut(), len: 0, cap: 0 };
            drachm_list_join(&mut out, &l, &sep);
            assert_eq!(bytes(&out as *const DrachmStr), b"ab, , cde");
            crate::drachm_str_drop(&mut out);
            let mut none = EMPTY_LIST;
            drachm_list_join(&mut out, &none, &sep);
            assert_eq!(bytes(&out as *const DrachmStr), b"");
            drachm_list_drop(&mut none, 24, None);
            drachm_list_drop(&mut l, 24, None);
        }
        assert_eq!(thread_live(), base);
    }
}
