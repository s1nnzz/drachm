use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, BuildHasherDefault, Hasher};

// u64 keys use the same SplitMix64 finalizer as Drachm.
#[derive(Default)]
struct FixedHasher(u64);

impl Hasher for FixedHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        let mut key = self.0 ^ value;
        key = (key ^ (key >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        key = (key ^ (key >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        self.0 = key ^ (key >> 31);
    }
}

fn workload(hasher: impl BuildHasher) -> u64 {
    let count = 5_000_000u64;
    let mut values = HashMap::with_hasher(hasher);
    for i in 0..count {
        values.insert(i, i * 3 + 1);
    }

    // A stride coprime to count visits every inserted key once.
    let mut key = 0;
    let mut total = 0;
    for _ in 0..count {
        total += values.get(&key).copied().unwrap_or(0);
        key += 7919;
        if key >= count {
            key -= count;
        }
    }
    total
}

fn main() {
    // bench/run.sh invokes the default mode. Pass --fixed to time it separately.
    let total = match std::env::args().nth(1).as_deref() {
        None => workload(RandomState::new()),
        Some("--fixed") => workload(BuildHasherDefault::<FixedHasher>::default()),
        Some(_) => {
            eprintln!("usage: map-rust [--fixed]");
            std::process::exit(2);
        }
    };
    println!("{}", total);
}
