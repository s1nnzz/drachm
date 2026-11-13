//! Maps: an insertion-ordered table of type-erased entries, the C side of
//! runtime/lists.h.
//!
//! `entries` holds one slot per insertion, in order. A slot is a 16-byte
//! header (the key's hash, then a live flag), the key at offset 16, the value
//! at `val_off`, and the whole slot padded to a multiple of 16 bytes, so every
//! field is aligned as long as the buffer is (malloc gives 16).
//!
//! `index` is an open-addressing table of u32 slot numbers (linear probing,
//! a power-of-two size, at least twice the slot count, so the load is at most
//! one half). A removed entry keeps its slot and its index position: its flag
//! is 0, so lookups pass over it like a tombstone and nothing else moves.
//! When the slots are full, a rebuild drops the dead ones, keeping the order,
//! and grows unless most of the slots are dead. Rebuilds move bytes only: the
//! stored hashes are reused, so no key is hashed again.

use std::ptr;

use crate::lists::{CopyFn, DropFn, EqFn, check_align, panic_text};
use crate::{counted_free, drachm_str_alloc};

/// Same layout as `drachm_map` in runtime/lists.h (48 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DrachmMap {
    pub entries: *mut u8,
    pub index: *mut u32,
    pub len: u64,
    pub cap: u64,
    pub live: u64,
    pub mask: u64,
}

pub(crate) const EMPTY_MAP: DrachmMap =
    DrachmMap { entries: ptr::null_mut(), index: ptr::null_mut(), len: 0, cap: 0, live: 0, mask: 0 };

/// Same layout as `drachm_map_ops` in runtime/lists.h.
#[repr(C)]
pub struct DrachmMapOps {
    pub ksize: u64,
    pub kalign: u64,
    pub vsize: u64,
    pub valign: u64,
    pub hash: Option<unsafe extern "C" fn(*const u8) -> u64>,
    pub eq: EqFn,
    pub drop_key: DropFn,
    pub drop_val: DropFn,
    pub copy_key: CopyFn,
    pub copy_val: CopyFn,
}

const HEADER: u64 = 16;
const EMPTY_SLOT: u32 = u32::MAX;
const MIN_CAP: u64 = 8;
/// Slot numbers are u32 with EMPTY_SLOT reserved; keep well inside that.
const MAX_CAP: u64 = 1 << 30;
const MAX_BYTES: u64 = isize::MAX as u64;

/// Byte offsets of a slot's parts, for one map shape.
#[derive(Clone, Copy)]
struct Layout {
    val_off: u64,
    stride: u64,
}

fn layout(ops: &DrachmMapOps) -> Layout {
    check_align(ops.kalign);
    check_align(ops.valign);
    let val_off = (HEADER + ops.ksize).div_ceil(ops.valign) * ops.valign;
    let stride = (val_off + ops.vsize).div_ceil(16) * 16;
    Layout { val_off, stride }
}

/// The slot at position `i`.
unsafe fn slot(entries: *mut u8, i: u64, stride: u64) -> *mut u8 {
    unsafe { entries.add((i * stride) as usize) }
}

unsafe fn slot_hash(e: *mut u8) -> u64 {
    unsafe { *(e as *const u64) }
}

unsafe fn slot_live(e: *mut u8) -> bool {
    unsafe { *(e.add(8) as *const u64) != 0 }
}

unsafe fn set_slot_header(e: *mut u8, hash: u64, live: bool) {
    unsafe {
        *(e as *mut u64) = hash;
        *(e.add(8) as *mut u64) = live as u64;
    }
}

unsafe fn key_of(e: *mut u8) -> *mut u8 {
    unsafe { e.add(HEADER as usize) }
}

unsafe fn val_of(e: *mut u8, lay: Layout) -> *mut u8 {
    unsafe { e.add(lay.val_off as usize) }
}

fn hash_key(ops: &DrachmMapOps, key: *const u8) -> u64 {
    match ops.hash {
        Some(f) => unsafe { f(key) },
        None => panic_text("map key has no hash"),
    }
}

fn keys_equal(ops: &DrachmMapOps, a: *const u8, b: *const u8) -> bool {
    match ops.eq {
        Some(f) => unsafe { f(a, b) != 0 },
        None => panic_text("map key has no equality"),
    }
}

/// Puts slot number `pos` (with hash `h`) at the first free index position on its probe path.
unsafe fn index_put(index: *mut u32, mask: u64, h: u64, pos: u64) {
    let mut s = h & mask;
    unsafe {
        while *index.add(s as usize) != EMPTY_SLOT {
            s = (s + 1) & mask;
        }
        *index.add(s as usize) = pos as u32;
    }
}

/// The slot holding a live entry equal to `key`, if any.
unsafe fn probe(m: &DrachmMap, key: *const u8, h: u64, lay: Layout, ops: &DrachmMapOps) -> Option<u64> {
    if m.index.is_null() {
        return None;
    }
    let mut s = h & m.mask;
    loop {
        let pos = unsafe { *m.index.add(s as usize) };
        if pos == EMPTY_SLOT {
            return None;
        }
        let e = unsafe { slot(m.entries, pos as u64, lay.stride) };
        if unsafe { slot_hash(e) } == h && unsafe { slot_live(e) } && keys_equal(ops, unsafe { key_of(e) }, key) {
            return Some(pos as u64);
        }
        s = (s + 1) & m.mask;
    }
}

/// A new, empty buffer pair for `cap` slots.
unsafe fn alloc_tables(cap: u64, lay: Layout) -> (*mut u8, *mut u32, u64) {
    let bytes = cap.checked_mul(lay.stride).filter(|&b| b <= MAX_BYTES).unwrap_or_else(|| panic_text("map is too big"));
    let entries = drachm_str_alloc(bytes);
    let slots = (cap * 2).next_power_of_two();
    let index = drachm_str_alloc(slots * 4) as *mut u32;
    for k in 0..slots {
        unsafe { *index.add(k as usize) = EMPTY_SLOT };
    }
    (entries, index, slots - 1)
}

/// Moves the live slots to a new table of `new_cap` slots, in order, and
/// frees the old one. Hashes are reused; nothing is called back.
unsafe fn rebuild(m: &mut DrachmMap, lay: Layout, new_cap: u64) {
    if new_cap > MAX_CAP {
        panic_text("map is too big");
    }
    let (entries, index, mask) = unsafe { alloc_tables(new_cap, lay) };
    let mut j = 0u64;
    for i in 0..m.len {
        let e = unsafe { slot(m.entries, i, lay.stride) };
        if unsafe { slot_live(e) } {
            let h = unsafe { slot_hash(e) };
            unsafe {
                ptr::copy_nonoverlapping(e, slot(entries, j, lay.stride), lay.stride as usize);
                index_put(index, mask, h, j);
            }
            j += 1;
        }
    }
    let (old_entries, old_index) = (m.entries, m.index);
    *m = DrachmMap { entries, index, len: j, cap: new_cap, live: m.live, mask };
    if !old_entries.is_null() {
        counted_free(old_entries);
        counted_free(old_index as *mut u8);
    }
}

/// Makes room for one more slot.
unsafe fn make_room(m: &mut DrachmMap, lay: Layout) {
    if m.cap == 0 {
        unsafe { rebuild(m, lay, MIN_CAP) };
    } else if m.len == m.cap {
        let dead = m.len - m.live;
        let new_cap = if dead * 2 > m.len { m.cap } else { m.cap * 2 };
        unsafe { rebuild(m, lay, new_cap) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_map_new(out: *mut DrachmMap) {
    unsafe { out.write(EMPTY_MAP) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_map_find(m: *const DrachmMap, key: *const u8, ops: *const DrachmMapOps) -> *mut u8 {
    let (m, ops) = unsafe { (&*m, &*ops) };
    if m.live == 0 {
        return ptr::null_mut();
    }
    let lay = layout(ops);
    let h = hash_key(ops, key);
    match unsafe { probe(m, key, h, lay, ops) } {
        Some(pos) => unsafe { val_of(slot(m.entries, pos, lay.stride), lay) },
        None => ptr::null_mut(),
    }
}

/// key and val are moved in. The NEW key is dropped when the key exists, and
/// the OLD value is dropped and replaced.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_map_insert(m: *mut DrachmMap, key: *mut u8, val: *mut u8, ops: *const DrachmMapOps) -> u8 {
    let (m, ops) = unsafe { (&mut *m, &*ops) };
    let lay = layout(ops);
    let h = hash_key(ops, key);
    if let Some(pos) = unsafe { probe(m, key, h, lay, ops) } {
        let e = unsafe { slot(m.entries, pos, lay.stride) };
        if let Some(f) = ops.drop_key {
            unsafe { f(key) };
        }
        let vp = unsafe { val_of(e, lay) };
        if let Some(f) = ops.drop_val {
            unsafe { f(vp) };
        }
        if ops.vsize > 0 {
            unsafe { ptr::copy_nonoverlapping(val, vp, ops.vsize as usize) };
        }
        return 0;
    }
    unsafe { make_room(m, lay) };
    let pos = m.len;
    let e = unsafe { slot(m.entries, pos, lay.stride) };
    unsafe {
        set_slot_header(e, h, true);
        if ops.ksize > 0 {
            ptr::copy_nonoverlapping(key, key_of(e), ops.ksize as usize);
        }
        if ops.vsize > 0 {
            ptr::copy_nonoverlapping(val, val_of(e, lay), ops.vsize as usize);
        }
        index_put(m.index, m.mask, h, pos);
    }
    m.len = pos + 1;
    m.live += 1;
    1
}

/// The stored key is dropped; the value is moved into out_val, or dropped
/// when out_val is NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_map_remove(m: *mut DrachmMap, key: *const u8, out_val: *mut u8, ops: *const DrachmMapOps) -> u8 {
    let (m, ops) = unsafe { (&mut *m, &*ops) };
    if m.live == 0 {
        return 0;
    }
    let lay = layout(ops);
    let h = hash_key(ops, key);
    let Some(pos) = (unsafe { probe(m, key, h, lay, ops) }) else { return 0 };
    let e = unsafe { slot(m.entries, pos, lay.stride) };
    // Dead first: the callbacks below must not see the entry as still there.
    unsafe { *(e.add(8) as *mut u64) = 0 };
    m.live -= 1;
    let (kp, vp) = unsafe { (key_of(e), val_of(e, lay)) };
    if !out_val.is_null() {
        if ops.vsize > 0 {
            unsafe { ptr::copy_nonoverlapping(vp, out_val, ops.vsize as usize) };
        }
    } else if let Some(f) = ops.drop_val {
        unsafe { f(vp) };
    }
    if let Some(f) = ops.drop_key {
        unsafe { f(kp) };
    }
    1
}

/// The panic of `m[k] += x` when `k` is not in the map.
#[unsafe(no_mangle)]
pub extern "C-unwind" fn drachm_map_key_panic() -> ! {
    panic_text("the key is not in the map, so there is no value to change")
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_map_len(m: *const DrachmMap) -> u64 {
    unsafe { (*m).live }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_map_next(
    m: *const DrachmMap,
    cursor: *mut u64,
    key: *mut *mut u8,
    val: *mut *mut u8,
    ops: *const DrachmMapOps,
) -> u8 {
    let (m, ops) = unsafe { (&*m, &*ops) };
    let lay = layout(ops);
    let mut i = unsafe { *cursor };
    while i < m.len {
        let e = unsafe { slot(m.entries, i, lay.stride) };
        i += 1;
        if unsafe { slot_live(e) } {
            unsafe {
                *cursor = i;
                *key = key_of(e);
                *val = val_of(e, lay);
            }
            return 1;
        }
    }
    unsafe { *cursor = i };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_map_drop(m: *mut DrachmMap, ops: *const DrachmMapOps) {
    let ops = unsafe { &*ops };
    let lay = layout(ops);
    let snap = unsafe { *m };
    // Empty first: the callbacks must not see the map as still there.
    unsafe { m.write(EMPTY_MAP) };
    for i in 0..snap.len {
        let e = unsafe { slot(snap.entries, i, lay.stride) };
        if unsafe { slot_live(e) } {
            if let Some(f) = ops.drop_key {
                unsafe { f(key_of(e)) };
            }
            if let Some(f) = ops.drop_val {
                unsafe { f(val_of(e, lay)) };
            }
        }
    }
    if !snap.entries.is_null() {
        counted_free(snap.entries);
        counted_free(snap.index as *mut u8);
    }
}

/// An independent deep copy with the same entries in the same order (removed
/// entries are not copied).
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_map_copy(out: *mut DrachmMap, m: *const DrachmMap, ops: *const DrachmMapOps) {
    let (src, ops) = unsafe { (*m, &*ops) };
    if src.live == 0 {
        unsafe { out.write(EMPTY_MAP) };
        return;
    }
    let lay = layout(ops);
    let cap = src.live;
    if cap > MAX_CAP {
        panic_text("map is too big");
    }
    let (entries, index, mask) = unsafe { alloc_tables(cap, lay) };
    let mut dst = DrachmMap { entries, index, len: 0, cap, live: src.live, mask };
    for i in 0..src.len {
        let e = unsafe { slot(src.entries, i, lay.stride) };
        if !unsafe { slot_live(e) } {
            continue;
        }
        let d = unsafe { slot(entries, dst.len, lay.stride) };
        let h = unsafe { slot_hash(e) };
        unsafe {
            set_slot_header(d, h, true);
            match ops.copy_key {
                Some(f) => f(key_of(d), key_of(e)),
                None => ptr::copy_nonoverlapping(key_of(e), key_of(d), ops.ksize as usize),
            }
            match ops.copy_val {
                Some(f) => f(val_of(d, lay), val_of(e, lay)),
                None => ptr::copy_nonoverlapping(val_of(e, lay), val_of(d, lay), ops.vsize as usize),
            }
            index_put(index, mask, h, dst.len);
        }
        dst.len += 1;
    }
    unsafe { out.write(dst) };
}

// ---- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{drachm_hash_bytes, drachm_hash_combine, drachm_hash_u64, drachm_str_hash};
    use crate::thread_live;
    use std::cell::Cell;

    thread_local! {
        static KEY_DROPS: Cell<u64> = const { Cell::new(0) };
        static VAL_DROPS: Cell<u64> = const { Cell::new(0) };
        static KEY_COPIES: Cell<u64> = const { Cell::new(0) };
    }

    unsafe extern "C" fn count_key_drop(_: *mut u8) {
        KEY_DROPS.with(|c| c.set(c.get() + 1));
    }
    unsafe extern "C" fn count_val_drop(_: *mut u8) {
        VAL_DROPS.with(|c| c.set(c.get() + 1));
    }
    unsafe extern "C" fn count_key_copy(dst: *mut u8, src: *const u8) {
        KEY_COPIES.with(|c| c.set(c.get() + 1));
        unsafe { ptr::copy_nonoverlapping(src, dst, 8) };
    }

    /// u64 keys: hash by the splitmix finaliser, compare by value.
    unsafe extern "C" fn hash_u64(p: *const u8) -> u64 {
        drachm_hash_u64(unsafe { *(p as *const u64) })
    }
    unsafe extern "C" fn eq_u64(a: *const u8, b: *const u8) -> u8 {
        (unsafe { *(a as *const u64) } == unsafe { *(b as *const u64) }) as u8
    }

    /// Byte keys of N bytes, hashed and compared as bytes.
    unsafe extern "C" fn hash_bytes_n<const N: usize>(p: *const u8) -> u64 {
        unsafe { drachm_hash_bytes(p, N as u64) }
    }
    unsafe extern "C" fn eq_bytes_n<const N: usize>(a: *const u8, b: *const u8) -> u8 {
        (unsafe { std::slice::from_raw_parts(a, N) } == unsafe { std::slice::from_raw_parts(b, N) }) as u8
    }

    /// Three-byte keys compared without case (`eq` is case-insensitive, so the hash must fold case too).
    unsafe extern "C" fn hash_ci(p: *const u8) -> u64 {
        let lower: Vec<u8> = unsafe { std::slice::from_raw_parts(p, 3) }.iter().map(|b| b.to_ascii_lowercase()).collect();
        unsafe { drachm_hash_bytes(lower.as_ptr(), 3) }
    }
    unsafe extern "C" fn eq_ci(a: *const u8, b: *const u8) -> u8 {
        let (a, b) = unsafe { (std::slice::from_raw_parts(a, 3), std::slice::from_raw_parts(b, 3)) };
        (a.eq_ignore_ascii_case(b)) as u8
    }

    const OPS_U64: DrachmMapOps = DrachmMapOps {
        ksize: 8,
        kalign: 8,
        vsize: 8,
        valign: 8,
        hash: Some(hash_u64),
        eq: Some(eq_u64),
        drop_key: None,
        drop_val: None,
        copy_key: None,
        copy_val: None,
    };

    fn ins(m: &mut DrachmMap, k: u64, v: u64) -> u8 {
        let (mut k, mut v) = (k, v);
        unsafe { drachm_map_insert(m, &mut k as *mut u64 as *mut u8, &mut v as *mut u64 as *mut u8, &OPS_U64) }
    }
    fn get(m: &DrachmMap, k: u64) -> Option<u64> {
        let p = unsafe { drachm_map_find(m, &k as *const u64 as *const u8, &OPS_U64) };
        (!p.is_null()).then(|| unsafe { *(p as *const u64) })
    }
    fn del(m: &mut DrachmMap, k: u64) -> Option<u64> {
        let mut out = 0u64;
        let hit = unsafe { drachm_map_remove(m, &k as *const u64 as *const u8, &mut out as *mut u64 as *mut u8, &OPS_U64) };
        (hit != 0).then_some(out)
    }
    /// The entries in iteration order.
    fn walk(m: &DrachmMap) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let mut cursor = 0u64;
        loop {
            let (mut k, mut v) = (ptr::null_mut::<u8>(), ptr::null_mut::<u8>());
            if unsafe { drachm_map_next(m, &mut cursor, &mut k, &mut v, &OPS_U64) } == 0 {
                break;
            }
            out.push(unsafe { (*(k as *const u64), *(v as *const u64)) });
        }
        out
    }

    #[test]
    fn zero_header_is_an_empty_map() {
        let mut m = DrachmMap { entries: ptr::null_mut(), index: ptr::null_mut(), len: 0, cap: 0, live: 0, mask: 0 };
        assert_eq!(get(&m, 1), None);
        assert_eq!(del(&mut m, 1), None);
        assert_eq!(walk(&m), vec![]);
        assert_eq!(unsafe { drachm_map_len(&m) }, 0);
        unsafe { drachm_map_drop(&mut m, &OPS_U64) };
        assert_eq!(ins(&mut m, 5, 50), 1);
        assert_eq!(get(&m, 5), Some(50));
        unsafe { drachm_map_drop(&mut m, &OPS_U64) };
    }

    #[test]
    fn a_hundred_thousand_inserts_removes_and_reinserts() {
        let base = thread_live();
        let mut m = EMPTY_MAP;
        for k in 0..100_000u64 {
            assert_eq!(ins(&mut m, k, k * 3), 1);
        }
        assert_eq!(unsafe { drachm_map_len(&m) }, 100_000);
        for k in (0..100_000u64).step_by(7) {
            assert_eq!(get(&m, k), Some(k * 3));
        }
        assert_eq!(get(&m, 100_000), None);
        for k in (0..100_000u64).filter(|k| k % 2 == 0) {
            assert_eq!(del(&mut m, k), Some(k * 3));
        }
        assert_eq!(unsafe { drachm_map_len(&m) }, 50_000);
        assert_eq!(get(&m, 4), None);
        assert_eq!(get(&m, 5), Some(15));
        for k in (0..100_000u64).filter(|k| k % 2 == 0).take(1000) {
            assert_eq!(ins(&mut m, k, k + 1), 1, "re-insert of a removed key is a new key");
        }
        // Odd keys in their original order, then the re-inserted evens in order.
        let order = walk(&m);
        assert_eq!(order.len(), 51_000);
        let keys: Vec<u64> = order.iter().map(|e| e.0).collect();
        let mut expect: Vec<u64> = (0..100_000u64).filter(|k| k % 2 == 1).collect();
        expect.extend((0..100_000u64).filter(|k| k % 2 == 0).take(1000));
        assert_eq!(keys, expect);
        assert_eq!(order.iter().find(|e| e.0 == 0), Some(&(0, 1)));
        unsafe { drachm_map_drop(&mut m, &OPS_U64) };
        assert_eq!(thread_live(), base);
    }

    /// Random inserts, replaces and removes checked against a Vec model that
    /// keeps insertion order. Small key range: plenty of tombstones, rebuilds
    /// and compactions.
    #[test]
    fn model_check_order_and_contents() {
        let base = thread_live();
        let mut m = EMPTY_MAP;
        let mut model: Vec<(u64, u64)> = Vec::new();
        let mut st = 12345u64;
        for step in 0..20_000u64 {
            st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let r = st >> 33;
            let k = (r % 300) as u64;
            match r % 5 {
                0 | 1 | 2 => {
                    let found = model.iter().position(|e| e.0 == k);
                    let fresh = ins(&mut m, k, step);
                    match found {
                        Some(p) => {
                            assert_eq!(fresh, 0);
                            model[p].1 = step;
                        }
                        None => {
                            assert_eq!(fresh, 1);
                            model.push((k, step));
                        }
                    }
                }
                3 => {
                    let found = model.iter().position(|e| e.0 == k);
                    assert_eq!(del(&mut m, k), found.map(|p| model.remove(p).1));
                }
                _ => {
                    assert_eq!(get(&m, k), model.iter().find(|e| e.0 == k).map(|e| e.1));
                }
            }
            assert_eq!(unsafe { drachm_map_len(&m) } as usize, model.len());
            if step % 997 == 0 {
                assert_eq!(walk(&m), model, "step {step}");
            }
        }
        assert_eq!(walk(&m), model);
        unsafe { drachm_map_drop(&mut m, &OPS_U64) };
        assert_eq!(thread_live(), base);
    }

    #[test]
    fn replacing_drops_the_new_key_and_the_old_value() {
        KEY_DROPS.with(|c| c.set(0));
        VAL_DROPS.with(|c| c.set(0));
        let ops = DrachmMapOps { drop_key: Some(count_key_drop), drop_val: Some(count_val_drop), ..OPS_U64 };
        let mut m = EMPTY_MAP;
        let (mut k, mut v) = (7u64, 70u64);
        unsafe {
            assert_eq!(drachm_map_insert(&mut m, &mut k as *mut u64 as *mut u8, &mut v as *mut u64 as *mut u8, &ops), 1);
            assert_eq!((KEY_DROPS.with(|c| c.get()), VAL_DROPS.with(|c| c.get())), (0, 0));
            let (mut k2, mut v2) = (7u64, 71u64);
            assert_eq!(drachm_map_insert(&mut m, &mut k2 as *mut u64 as *mut u8, &mut v2 as *mut u64 as *mut u8, &ops), 0);
        }
        assert_eq!((KEY_DROPS.with(|c| c.get()), VAL_DROPS.with(|c| c.get())), (1, 1));
        assert_eq!(get(&m, 7), Some(71));
        assert_eq!(unsafe { drachm_map_len(&m) }, 1);
        unsafe {
            // Remove with the value wanted: moved out, the key dropped.
            let mut out = 0u64;
            assert_eq!(drachm_map_remove(&mut m, &7u64 as *const u64 as *const u8, &mut out as *mut u64 as *mut u8, &ops), 1);
            assert_eq!(out, 71);
            assert_eq!((KEY_DROPS.with(|c| c.get()), VAL_DROPS.with(|c| c.get())), (2, 1));
            // Remove with the value not wanted: the value is dropped.
            ins(&mut m, 8, 80);
            assert_eq!(drachm_map_remove(&mut m, &8u64 as *const u64 as *const u8, ptr::null_mut(), &ops), 1);
            assert_eq!((KEY_DROPS.with(|c| c.get()), VAL_DROPS.with(|c| c.get())), (3, 2));
            drachm_map_drop(&mut m, &ops);
        }
        assert_eq!((KEY_DROPS.with(|c| c.get()), VAL_DROPS.with(|c| c.get())), (3, 2));
    }

    #[test]
    fn drop_at_the_end_drops_every_live_entry_once() {
        let base = thread_live();
        KEY_DROPS.with(|c| c.set(0));
        VAL_DROPS.with(|c| c.set(0));
        let ops = DrachmMapOps { drop_key: Some(count_key_drop), drop_val: Some(count_val_drop), ..OPS_U64 };
        let mut m = EMPTY_MAP;
        for k in 0..100u64 {
            let (mut kk, mut vv) = (k, k);
            unsafe { drachm_map_insert(&mut m, &mut kk as *mut u64 as *mut u8, &mut vv as *mut u64 as *mut u8, &ops) };
        }
        for k in 0..40u64 {
            unsafe { drachm_map_remove(&mut m, &k as *const u64 as *const u8, ptr::null_mut(), &ops) };
        }
        assert_eq!((KEY_DROPS.with(|c| c.get()), VAL_DROPS.with(|c| c.get())), (40, 40));
        unsafe { drachm_map_drop(&mut m, &ops) };
        assert_eq!((KEY_DROPS.with(|c| c.get()), VAL_DROPS.with(|c| c.get())), (100, 100));
        assert_eq!(thread_live(), base);
    }

    #[test]
    fn custom_equality_for_case_insensitive_keys() {
        let base = thread_live();
        let ops = DrachmMapOps {
            ksize: 3,
            kalign: 1,
            vsize: 8,
            valign: 8,
            hash: Some(hash_ci),
            eq: Some(eq_ci),
            drop_key: None,
            drop_val: None,
            copy_key: None,
            copy_val: None,
        };
        let mut m = EMPTY_MAP;
        let put = |m: &mut DrachmMap, k: &[u8; 3], v: u64| {
            let mut v = v;
            let mut kk = *k;
            unsafe { drachm_map_insert(m, kk.as_mut_ptr(), &mut v as *mut u64 as *mut u8, &ops) }
        };
        unsafe {
            assert_eq!(put(&mut m, b"abc", 1), 1);
            assert_eq!(put(&mut m, b"ABC", 2), 0, "equal by case: a replacement");
            assert_eq!(put(&mut m, b"xyz", 3), 1);
            assert_eq!(drachm_map_len(&m), 2);
            let p = drachm_map_find(&m, b"aBc".as_ptr(), &ops);
            assert_eq!(*(p as *const u64), 2);
            assert!(drachm_map_find(&m, b"abd".as_ptr(), &ops).is_null());
            drachm_map_drop(&mut m, &ops);
        }
        assert_eq!(thread_live(), base);
    }

    #[test]
    fn copy_is_independent_and_keeps_order() {
        let base = thread_live();
        KEY_COPIES.with(|c| c.set(0));
        let mut m = EMPTY_MAP;
        for k in 0..30u64 {
            ins(&mut m, k, k * 10);
        }
        for k in (0..30u64).step_by(4) {
            del(&mut m, k);
        }
        let ops = DrachmMapOps { copy_key: Some(count_key_copy), ..OPS_U64 };
        let mut copy = EMPTY_MAP;
        unsafe { drachm_map_copy(&mut copy, &m, &ops) };
        assert_eq!(KEY_COPIES.with(|c| c.get()), 22, "one copy per live entry");
        assert_eq!(walk(&copy), walk(&m));
        // Changing the original leaves the copy alone.
        ins(&mut m, 1, 999);
        del(&mut m, 2);
        ins(&mut m, 100, 1);
        assert_eq!(get(&copy, 1), Some(10));
        assert_eq!(get(&copy, 2), Some(20));
        assert_eq!(get(&copy, 100), None);
        assert_eq!(unsafe { drachm_map_len(&copy) }, 22);
        unsafe {
            drachm_map_drop(&mut m, &OPS_U64);
            drachm_map_drop(&mut copy, &OPS_U64);
        }
        assert_eq!(thread_live(), base);
    }

    /// 1-byte keys with 40-byte values (both alignments), 24-byte keys with
    /// zero-sized values (a set): the slot arithmetic under odd shapes.
    #[test]
    fn odd_shapes_round_trip() {
        let base = thread_live();
        let ops1 = DrachmMapOps {
            ksize: 1,
            kalign: 1,
            vsize: 40,
            valign: 8,
            hash: Some(hash_bytes_n::<1>),
            eq: Some(eq_bytes_n::<1>),
            drop_key: None,
            drop_val: None,
            copy_key: None,
            copy_val: None,
        };
        let mut m = EMPTY_MAP;
        for k in 0..=255u8 {
            let mut key = k;
            let mut val = [k as u64; 5];
            assert_eq!(unsafe { drachm_map_insert(&mut m, &mut key, val.as_mut_ptr() as *mut u8, &ops1) }, 1);
        }
        for k in 0..=255u8 {
            let p = unsafe { drachm_map_find(&m, &k, &ops1) } as *const [u64; 5];
            assert_eq!(unsafe { *p }, [k as u64; 5]);
            assert_eq!(p as usize % 8, 0);
        }
        unsafe { drachm_map_drop(&mut m, &ops1) };

        let set24 = DrachmMapOps {
            ksize: 24,
            kalign: 8,
            vsize: 0,
            valign: 1,
            hash: Some(hash_bytes_n::<24>),
            eq: Some(eq_bytes_n::<24>),
            drop_key: None,
            drop_val: None,
            copy_key: None,
            copy_val: None,
        };
        let mut s = EMPTY_MAP;
        for k in 0..1000u64 {
            let mut key = [k as u8; 24];
            key[8..16].copy_from_slice(&k.to_le_bytes());
            assert_eq!(unsafe { drachm_map_insert(&mut s, key.as_mut_ptr(), ptr::null_mut(), &set24) }, 1);
        }
        let mut probe_key = [500u64 as u8; 24];
        probe_key[8..16].copy_from_slice(&500u64.to_le_bytes());
        assert!(!unsafe { drachm_map_find(&s, probe_key.as_ptr(), &set24) }.is_null());
        assert_eq!(unsafe { drachm_map_len(&s) }, 1000);
        unsafe { drachm_map_drop(&mut s, &set24) };
        assert_eq!(thread_live(), base);
    }

    #[test]
    fn float_style_keys_compare_by_bits() {
        // Keys are the bit patterns: NaN equals itself, 0.0 and -0.0 differ.
        unsafe extern "C" fn hash_bits(p: *const u8) -> u64 {
            drachm_hash_u64(unsafe { *(p as *const u64) })
        }
        let ops = DrachmMapOps { hash: Some(hash_bits), ..OPS_U64 };
        let mut m = EMPTY_MAP;
        let nan = f64::NAN.to_bits();
        let mut k = nan;
        let mut v = 1u64;
        unsafe {
            assert_eq!(drachm_map_insert(&mut m, &mut k as *mut u64 as *mut u8, &mut v as *mut u64 as *mut u8, &ops), 1);
            let p = drachm_map_find(&m, &nan as *const u64 as *const u8, &ops);
            assert!(!p.is_null());
            let zero = 0.0f64.to_bits();
            let neg = (-0.0f64).to_bits();
            let mut z = zero;
            let mut vv = 2u64;
            assert_eq!(drachm_map_insert(&mut m, &mut z as *mut u64 as *mut u8, &mut vv as *mut u64 as *mut u8, &ops), 1);
            assert!(drachm_map_find(&m, &neg as *const u64 as *const u8, &ops).is_null());
            drachm_map_drop(&mut m, &ops);
        }
    }

    #[test]
    fn hashes_are_fixed_and_string_hash_follows_bytes() {
        assert_ne!(drachm_hash_combine(1, 2), drachm_hash_combine(2, 1));
        assert_ne!(drachm_hash_u64(0), drachm_hash_u64(1));
        assert_eq!(unsafe { drachm_hash_bytes(b"".as_ptr(), 0) }, unsafe { drachm_hash_bytes(b"".as_ptr(), 0) });
        let a = b"hello world, a longer key";
        assert_eq!(unsafe { drachm_hash_bytes(a.as_ptr(), a.len() as u64) }, unsafe { drachm_hash_bytes(a.as_ptr(), a.len() as u64) });
        assert_ne!(unsafe { drachm_hash_bytes(a.as_ptr(), 5) }, unsafe { drachm_hash_bytes(a.as_ptr(), 6) });
        let s = crate::DrachmStr { ptr: a.as_ptr() as *mut u8, len: a.len() as u64, cap: 0 };
        assert_eq!(unsafe { drachm_str_hash(&s) }, unsafe { drachm_hash_bytes(a.as_ptr(), a.len() as u64) });
    }
}
