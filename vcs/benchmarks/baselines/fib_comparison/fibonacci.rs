//! Fibonacci Benchmark - Rust Implementation
//!
//! Baseline implementation for comparison with Verum, C, and Go.
//! Build with: cargo build --release

use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

const WARMUP_ITERATIONS: usize = 10;

/// Recursive Fibonacci
fn fib_recursive(n: i32) -> i64 {
    if n <= 1 {
        n as i64
    } else {
        fib_recursive(n - 1) + fib_recursive(n - 2)
    }
}

/// Iterative Fibonacci
fn fib_iterative(n: i32) -> i64 {
    if n <= 1 {
        return n as i64;
    }

    let (mut a, mut b) = (0i64, 1i64);
    for _ in 2..=n {
        let temp = a + b;
        a = b;
        b = temp;
    }
    b
}

/// Memoized Fibonacci
fn fib_memoized(n: i32, cache: &mut HashMap<i32, i64>) -> i64 {
    if n <= 1 {
        return n as i64;
    }

    if let Some(&result) = cache.get(&n) {
        return result;
    }

    let result = fib_memoized(n - 1, cache) + fib_memoized(n - 2, cache);
    cache.insert(n, result);
    result
}

/// Matrix multiplication for 2x2 matrices
fn matrix_mult(a: [[i64; 2]; 2], b: [[i64; 2]; 2]) -> [[i64; 2]; 2] {
    [
        [
            a[0][0] * b[0][0] + a[0][1] * b[1][0],
            a[0][0] * b[0][1] + a[0][1] * b[1][1],
        ],
        [
            a[1][0] * b[0][0] + a[1][1] * b[1][0],
            a[1][0] * b[0][1] + a[1][1] * b[1][1],
        ],
    ]
}

/// Matrix exponentiation Fibonacci O(log n)
fn fib_matrix(n: i32) -> i64 {
    if n <= 1 {
        return n as i64;
    }

    let mut result = [[1i64, 0i64], [0i64, 1i64]]; // Identity
    let mut base = [[1i64, 1i64], [1i64, 0i64]];
    let mut exp = n;

    while exp > 0 {
        if exp % 2 == 1 {
            result = matrix_mult(result, base);
        }
        base = matrix_mult(base, base);
        exp /= 2;
    }

    result[0][1]
}

fn benchmark_recursive_30() {
    const N: i32 = 30;
    const EXPECTED: i64 = 832040;
    const ITERATIONS: usize = 100;

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        let result = fib_recursive(N);
        assert_eq!(result, EXPECTED);
    }

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(fib_recursive(N));
    }
    let elapsed = start.elapsed();

    let per_call_ms = elapsed.as_secs_f64() * 1000.0 / ITERATIONS as f64;
    println!("[Rust] fib_recursive_30: {:.3} ms/call", per_call_ms);
}

fn benchmark_recursive_40() {
    const N: i32 = 40;
    const EXPECTED: i64 = 102334155;
    const ITERATIONS: usize = 3;

    // Warmup
    let result = fib_recursive(N);
    assert_eq!(result, EXPECTED);

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(fib_recursive(N));
    }
    let elapsed = start.elapsed();

    let per_call_ms = elapsed.as_secs_f64() * 1000.0 / ITERATIONS as f64;
    println!("[Rust] fib_recursive_40: {:.3} ms/call", per_call_ms);
}

fn benchmark_iterative_45() {
    const N: i32 = 45;
    const ITERATIONS: usize = 10_000_000;

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        black_box(fib_iterative(N));
    }

    let start = Instant::now();
    let mut sum = 0i64;
    for _ in 0..ITERATIONS {
        sum += fib_iterative(N);
    }
    black_box(sum);
    let elapsed = start.elapsed();

    let per_call_ns = elapsed.as_nanos() as f64 / ITERATIONS as f64;
    println!("[Rust] fib_iterative_45: {:.3} ns/call", per_call_ns);
}

fn benchmark_iterative_90() {
    const N: i32 = 90;
    const ITERATIONS: usize = 1_000_000;

    let start = Instant::now();
    let mut sum = 0i64;
    for _ in 0..ITERATIONS {
        sum += fib_iterative(N);
    }
    black_box(sum);
    let elapsed = start.elapsed();

    let per_call_ns = elapsed.as_nanos() as f64 / ITERATIONS as f64;
    println!("[Rust] fib_iterative_90: {:.3} ns/call", per_call_ns);
}

fn benchmark_memoized_40() {
    const N: i32 = 40;
    const ITERATIONS: usize = 10_000;

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        let mut cache = HashMap::new();
        black_box(fib_memoized(N, &mut cache));
    }

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let mut cache = HashMap::new();
        black_box(fib_memoized(N, &mut cache));
    }
    let elapsed = start.elapsed();

    let per_call_us = elapsed.as_micros() as f64 / ITERATIONS as f64;
    println!("[Rust] fib_memoized_40: {:.3} us/call", per_call_us);
}

fn benchmark_matrix_1000() {
    const N: i32 = 1000;
    const ITERATIONS: usize = 1_000_000;

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        black_box(fib_matrix(N));
    }

    let start = Instant::now();
    let mut sum = 0i64;
    for _ in 0..ITERATIONS {
        sum += fib_matrix(N);
    }
    black_box(sum);
    let elapsed = start.elapsed();

    let per_call_ns = elapsed.as_nanos() as f64 / ITERATIONS as f64;
    println!("[Rust] fib_matrix_1000: {:.3} ns/call", per_call_ns);
}

fn main() {
    println!("=== Fibonacci Benchmark - Rust ===\n");

    benchmark_recursive_30();
    benchmark_recursive_40();
    benchmark_iterative_45();
    benchmark_iterative_90();
    benchmark_memoized_40();
    benchmark_matrix_1000();

    println!("\nAll benchmarks completed.");
}
