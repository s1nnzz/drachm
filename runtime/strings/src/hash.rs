//! Hash helpers for map keys (runtime/lists.h, "hashing"). Fixed constants and
//! no per-process seed: the same bytes give the same 64-bit value on every
//! run, on both backends and on every platform. Byte words are read
//! little-endian explicitly, so big-endian hosts agree too.

use crate::DrachmStr;

/// splitmix64's finaliser: a bijection on u64 with good bit mixing.
#[unsafe(no_mangle)]
pub extern "C-unwind" fn drachm_hash_u64(x: u64) -> u64 {
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Mixes `x` into the running hash `h` (order matters: combine(a, b) differs from combine(b, a)).
#[unsafe(no_mangle)]
pub extern "C-unwind" fn drachm_hash_combine(h: u64, x: u64) -> u64 {
    drachm_hash_u64(h ^ x.wrapping_add(0x9e37_79b9_7f4a_7c15).wrapping_add(h << 6).wrapping_add(h >> 2))
}

/// Hashes `n` bytes at `p` eight at a time, then the tail, then the length.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_hash_bytes(p: *const u8, n: u64) -> u64 {
    let bytes: &[u8] = if n == 0 { &[] } else { unsafe { std::slice::from_raw_parts(p, n as usize) } };
    let mut h = 0x243f_6a88_85a3_08d3_u64; // a fixed seed (pi's digits)
    let mut words = bytes.chunks_exact(8);
    for w in &mut words {
        h = drachm_hash_combine(h, u64::from_le_bytes(w.try_into().unwrap()));
    }
    let tail = words.remainder();
    if !tail.is_empty() {
        let mut buf = [0u8; 8];
        buf[..tail.len()].copy_from_slice(tail);
        h = drachm_hash_combine(h, u64::from_le_bytes(buf));
    }
    drachm_hash_u64(h ^ n)
}

/// The hash of a string's bytes; equal strings (`drachm_str_eq`) hash equal.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_str_hash(s: *const DrachmStr) -> u64 {
    let s = unsafe { &*s };
    unsafe { drachm_hash_bytes(s.ptr, s.len) }
}
