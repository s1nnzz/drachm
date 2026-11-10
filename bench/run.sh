#!/bin/sh
# Compare Drachm (-O) against C (cc -O2) and Rust (rustc -O) on each benchmark.
# Prints the best of 5 wall-clock times per program. Usage: bench/run.sh
set -e
cd "$(dirname "$0")/.."
cargo build --release --quiet
out=target/bench
mkdir -p "$out"

best() {
    best=""
    for _ in 1 2 3 4 5; do
        s=$(date +%s%N)
        "$1" > /dev/null
        e=$(date +%s%N)
        t=$(( (e - s) / 1000000 ))
        if [ -z "$best" ] || [ "$t" -lt "$best" ]; then best=$t; fi
    done
    echo "$best"
}

printf '%-10s %10s %10s %10s %8s\n' bench drachm c rust "vs c"
for src in bench/*.dr*; do
    name=$(basename "$src"); name=${name%.*}
    target/release/drachm build "$src" -O -o "$out/$name-drachm"
    cc -O2 "bench/$name.c" -o "$out/$name-c"
    rustc -O "bench/$name.rs" -o "$out/$name-rust" 2>/dev/null
    for impl in c rust; do
        [ "$("$out/$name-drachm")" = "$("$out/$name-$impl")" ] || { echo "$name: drachm and $impl disagree"; exit 1; }
    done
    d=$(best "$out/$name-drachm"); c=$(best "$out/$name-c"); r=$(best "$out/$name-rust")
    printf '%-10s %8sms %8sms %8sms %7s%%\n' "$name" "$d" "$c" "$r" $(( d * 100 / c ))
done
