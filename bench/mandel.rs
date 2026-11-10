fn escape(cr: f64, ci: f64) -> u32 {
    let (mut zr, mut zi) = (0.0f64, 0.0f64);
    let mut i = 0u32;
    while i < 200 && zr * zr + zi * zi <= 4.0 {
        let t = zr * zr - zi * zi + cr;
        zi = 2.0 * zr * zi + ci;
        zr = t;
        i += 1;
    }
    i
}

fn main() {
    let size = std::hint::black_box(1200u32);
    let mut total = 0u64;
    for y in 0..size {
        for x in 0..size {
            let cr = x as f64 * 3.0 / size as f64 - 2.0;
            let ci = y as f64 * 2.0 / size as f64 - 1.0;
            total += escape(cr, ci) as u64;
        }
    }
    println!("{total}");
}
