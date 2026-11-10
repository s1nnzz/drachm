fn main() {
    let (mut total, mut hits) = (0u64, 0u64);
    let mut tail = String::new();
    let mut i = 0u32;
    while i < std::hint::black_box(2_000_000) {
        let line = format!("item {i}: ") + &"ab".repeat((i % 7) as usize);
        total += line.chars().count() as u64;
        if line.contains("99") {
            hits += 1;
        }
        tail.push(line.chars().next_back().unwrap());
        i += 1;
    }
    println!("{total}\n{hits}\n{}\n{}", tail.matches('b').count(), tail.chars().count());
}
