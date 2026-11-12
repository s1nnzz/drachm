trait Shape {
    fn area(&self) -> u64;
}

struct Square {
    side: u64,
}

struct Strip {
    len: u64,
}

impl Shape for Square {
    fn area(&self) -> u64 {
        self.side * self.side
    }
}

impl Shape for Strip {
    fn area(&self) -> u64 {
        self.len + 3
    }
}

#[inline(never)]
fn total(a: &dyn Shape, b: &dyn Shape, pairs: u64) -> u64 {
    let mut sum = 0u64;
    for _ in 0..pairs {
        let s = if sum % 3 == 0 { b } else { a };
        sum = sum.wrapping_add(s.area());
    }
    sum
}

fn main() {
    let q = Square { side: std::hint::black_box(7) };
    let s = Strip { len: std::hint::black_box(11) };
    println!("{}", total(&q, &s, 100_000_000));
}
