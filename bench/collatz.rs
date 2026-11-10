fn steps(mut n: u64) -> u32 {
    let mut count = 0u32;
    while n != 1 {
        if n & 1 == 0 {
            n >>= 1;
        } else {
            n = n.wrapping_mul(3).wrapping_add(1);
        }
        count += 1;
    }
    count
}

fn main() {
    let (mut best, mut best_steps) = (0u64, 0u32);
    let mut i = 1u64;
    while i < std::hint::black_box(3_000_000) {
        let s = steps(i);
        if s > best_steps {
            best_steps = s;
            best = i;
        }
        i += 1;
    }
    println!("{best}\n{best_steps}");
}
