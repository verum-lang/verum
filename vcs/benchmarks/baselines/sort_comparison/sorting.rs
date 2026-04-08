//! Sorting Algorithm Benchmark - Rust Implementation
//!
//! Baseline implementation for comparison with Verum, C, and Go.
//! Build with: cargo build --release

use std::hint::black_box;
use std::time::Instant;

const WARMUP_ITERATIONS: usize = 5;

/// Simple xorshift64 RNG
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> i64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state as i64
    }
}

/// Generate random array
fn generate_random_array(size: usize, seed: u64) -> Vec<i64> {
    let mut rng = Rng::new(seed);
    (0..size).map(|_| rng.next()).collect()
}

/// Generate sorted array
fn generate_sorted_array(size: usize) -> Vec<i64> {
    (0..size as i64).collect()
}

/// Generate reverse-sorted array
fn generate_reverse_array(size: usize) -> Vec<i64> {
    (0..size as i64).rev().collect()
}

fn benchmark_sort_100() {
    const SIZE: usize = 100;
    const ITERATIONS: usize = 100_000;

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        let mut arr = generate_random_array(SIZE, 42);
        arr.sort();
    }

    let start = Instant::now();
    for i in 0..ITERATIONS {
        let mut arr = generate_random_array(SIZE, i as u64);
        arr.sort();
        black_box(&arr);
    }
    let elapsed = start.elapsed();

    let per_sort_us = elapsed.as_micros() as f64 / ITERATIONS as f64;
    println!("[Rust] sort_100: {:.3} us/sort", per_sort_us);
}

fn benchmark_sort_1000() {
    const SIZE: usize = 1000;
    const ITERATIONS: usize = 10_000;

    let start = Instant::now();
    for i in 0..ITERATIONS {
        let mut arr = generate_random_array(SIZE, i as u64);
        arr.sort();
        black_box(&arr);
    }
    let elapsed = start.elapsed();

    let per_sort_us = elapsed.as_micros() as f64 / ITERATIONS as f64;
    println!("[Rust] sort_1000: {:.3} us/sort", per_sort_us);
}

fn benchmark_sort_10000() {
    const SIZE: usize = 10_000;
    const ITERATIONS: usize = 1_000;

    let start = Instant::now();
    for i in 0..ITERATIONS {
        let mut arr = generate_random_array(SIZE, i as u64);
        arr.sort();
        black_box(&arr);
    }
    let elapsed = start.elapsed();

    let per_sort_us = elapsed.as_micros() as f64 / ITERATIONS as f64;
    println!("[Rust] sort_10000: {:.3} us/sort", per_sort_us);
}

fn benchmark_sort_100000() {
    const SIZE: usize = 100_000;
    const ITERATIONS: usize = 100;

    let start = Instant::now();
    for i in 0..ITERATIONS {
        let mut arr = generate_random_array(SIZE, i as u64);
        arr.sort();
        black_box(&arr);
    }
    let elapsed = start.elapsed();

    let per_sort_ms = elapsed.as_millis() as f64 / ITERATIONS as f64;
    println!("[Rust] sort_100000: {:.3} ms/sort", per_sort_ms);
}

fn benchmark_sort_1000000() {
    const SIZE: usize = 1_000_000;
    const ITERATIONS: usize = 10;

    let start = Instant::now();
    for i in 0..ITERATIONS {
        let mut arr = generate_random_array(SIZE, i as u64);
        arr.sort();
        black_box(&arr);
    }
    let elapsed = start.elapsed();

    let per_sort_ms = elapsed.as_millis() as f64 / ITERATIONS as f64;
    println!("[Rust] sort_1000000: {:.3} ms/sort", per_sort_ms);
}

fn benchmark_sort_sorted() {
    const SIZE: usize = 100_000;
    const ITERATIONS: usize = 100;

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let mut arr = generate_sorted_array(SIZE);
        arr.sort();
        black_box(&arr);
    }
    let elapsed = start.elapsed();

    let per_sort_ms = elapsed.as_millis() as f64 / ITERATIONS as f64;
    println!("[Rust] sort_sorted_100k: {:.3} ms/sort", per_sort_ms);
}

fn benchmark_sort_reverse() {
    const SIZE: usize = 100_000;
    const ITERATIONS: usize = 100;

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let mut arr = generate_reverse_array(SIZE);
        arr.sort();
        black_box(&arr);
    }
    let elapsed = start.elapsed();

    let per_sort_ms = elapsed.as_millis() as f64 / ITERATIONS as f64;
    println!("[Rust] sort_reverse_100k: {:.3} ms/sort", per_sort_ms);
}

fn benchmark_sort_unstable() {
    const SIZE: usize = 100_000;
    const ITERATIONS: usize = 50;

    let start = Instant::now();
    for i in 0..ITERATIONS {
        let mut arr = generate_random_array(SIZE, i as u64);
        arr.sort_unstable();
        black_box(&arr);
    }
    let elapsed = start.elapsed();

    let per_sort_ms = elapsed.as_millis() as f64 / ITERATIONS as f64;
    println!("[Rust] sort_unstable_100k: {:.3} ms/sort", per_sort_ms);
}

fn main() {
    println!("=== Sorting Benchmark - Rust ===\n");

    benchmark_sort_100();
    benchmark_sort_1000();
    benchmark_sort_10000();
    benchmark_sort_100000();
    benchmark_sort_1000000();
    benchmark_sort_sorted();
    benchmark_sort_reverse();
    benchmark_sort_unstable();

    println!("\nAll benchmarks completed.");
}
