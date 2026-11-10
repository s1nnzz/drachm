fn is_prime(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    let mut d = 2;
    while d * d <= n {
        if n % d == 0 {
            return false;
        }
        d += 1;
    }
    true
}

fn main() {
    let mut count = 0u64;
    let mut i = 0u64;
    while i < std::hint::black_box(5_000_000) {
        if is_prime(i) {
            count += 1;
        }
        i += 1;
    }
    println!("{count}");
}
