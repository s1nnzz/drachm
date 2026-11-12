// Failing calls, Rust version: Result<u64, ()> and Result<String, String>.
#[inline(never)]
fn step(x: u64) -> Result<u64, ()> {
    if x % 1000003 == 7 {
        return Err(());
    }
    Ok((x * 3 + 1) % 1000003)
}

#[inline(never)]
fn plain(x: u64) -> u64 {
    if x % 1000003 == 7 {
        return 0;
    }
    (x * 3 + 1) % 1000003
}

#[inline(never)]
fn level3(x: u64) -> Result<u64, ()> {
    if x % 10000 == 7 {
        return Err(());
    }
    Ok(x + 1)
}

#[inline(never)]
fn level2(x: u64) -> Result<u64, ()> {
    let a = level3(x)?;
    Ok(a + 1)
}

#[inline(never)]
fn level1(x: u64) -> Result<u64, ()> {
    let a = level2(x)?;
    Ok(a + 1)
}

#[inline(never)]
fn label(i: u64) -> Result<String, String> {
    if i % 100 == 99 {
        return Err(i.to_string());
    }
    Ok(i.to_string())
}

fn main() {
    let mut total: u64 = 0;
    for i in 0..100_000_000u64 {
        total += step(i).unwrap_or(0);
    }
    println!("{total}");

    total = 0;
    for i in 0..20_000_000u64 {
        total += level1(i).unwrap_or(0);
    }
    println!("{total}");

    total = 0;
    for i in 0..2_000_000u64 {
        total += label(i).unwrap_or_else(|_| "-".to_string()).len() as u64;
    }
    println!("{total}");

    total = 0;
    for i in 0..100_000_000u64 {
        total += plain(i);
    }
    println!("{total}");
}
