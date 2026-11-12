fn biggest(a: u64, b: u64) -> u64 {
    if a > b { a } else { b }
}

fn main_sum(n: u64) -> u64 {
    let mut xs: Vec<u64> = Vec::new();
    let mut i = 0;
    while i < n {
        xs.push(i * 3 + 1);
        i += 1;
    }
    let mut total = 0u64;
    let mut j = 0;
    while j < xs.len() {
        total = total.wrapping_add(xs[j]);
        j += 1;
    }
    total
}

fn main_index(n: u64, reads: u64) -> u64 {
    let mut xs: Vec<u64> = Vec::new();
    let mut i = 0;
    while i < n {
        xs.push(i ^ 5);
        i += 1;
    }
    let mut state = 12345u64;
    let mut total = 0u64;
    let mut k = 0;
    while k < reads {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        total = total.wrapping_add(xs[((state >> 33) % n) as usize]);
        k += 1;
    }
    total
}

fn main_generic(n: u64) -> u64 {
    let mut xs: Vec<u64> = Vec::new();
    let mut i = 0;
    while i < n {
        xs.push((i * 7919) % 10007);
        i += 1;
    }
    let mut best = 0u64;
    let mut j = 0;
    while j < xs.len() {
        best = biggest(best, xs[j]).wrapping_add(xs[j] & 1);
        j += 1;
    }
    best
}

fn main() {
    println!("{}", main_sum(20000000));
    println!("{}", main_index(1000000, 50000000));
    println!("{}", main_generic(20000000));
}
