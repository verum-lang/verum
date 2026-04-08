//! CBGR Check Baseline - Rust Implementation
//!
//! This benchmark measures array access in Rust with its built-in
//! bounds checking as a comparison point for Verum's CBGR system.
//!
//! Compile: rustc -O -o cbgr_check cbgr_check.rs

use std::hint::black_box;
use std::time::Instant;

const ITERATIONS: usize = 10_000_000;
const ARRAY_SIZE: usize = 1000;

fn benchmark_array_access_safe(data: &[i64]) -> i64 {
    let mut sum = 0i64;
    for _ in 0..ITERATIONS {
        for j in 0..data.len() {
            sum = sum.wrapping_add(data[j]);
        }
    }
    sum
}

fn benchmark_array_access_unchecked(data: &[i64]) -> i64 {
    let mut sum = 0i64;
    for _ in 0..ITERATIONS {
        for j in 0..data.len() {
            // SAFETY: j is always in bounds
            sum = sum.wrapping_add(unsafe { *data.get_unchecked(j) });
        }
    }
    sum
}

fn benchmark_iterator(data: &[i64]) -> i64 {
    let mut sum = 0i64;
    for _ in 0..ITERATIONS {
        for &val in data.iter() {
            sum = sum.wrapping_add(val);
        }
    }
    sum
}

fn benchmark_slice_bounds(data: &[i64]) -> i64 {
    let mut sum = 0i64;
    for _ in 0..ITERATIONS {
        let slice = &data[0..ARRAY_SIZE];
        for j in 0..slice.len() {
            sum = sum.wrapping_add(slice[j]);
        }
    }
    sum
}

fn main() {
    let data: Vec<i64> = (0..ARRAY_SIZE as i64).collect();

    // Warmup
    black_box(benchmark_array_access_safe(&data));

    // Safe array access (with bounds check)
    let start = Instant::now();
    let result = benchmark_array_access_safe(&data);
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / (ITERATIONS * ARRAY_SIZE) as f64;
    println!(
        "Array access (safe):       {:.2} ns/op (result: {})",
        ns_per_op,
        black_box(result)
    );

    // Unchecked access
    let start = Instant::now();
    let result = benchmark_array_access_unchecked(&data);
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / (ITERATIONS * ARRAY_SIZE) as f64;
    println!(
        "Array access (unchecked):  {:.2} ns/op (result: {})",
        ns_per_op,
        black_box(result)
    );

    // Iterator (bounds-free)
    let start = Instant::now();
    let result = benchmark_iterator(&data);
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / (ITERATIONS * ARRAY_SIZE) as f64;
    println!(
        "Iterator access:           {:.2} ns/op (result: {})",
        ns_per_op,
        black_box(result)
    );

    // Slice bounds
    let start = Instant::now();
    let result = benchmark_slice_bounds(&data);
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / (ITERATIONS * ARRAY_SIZE) as f64;
    println!(
        "Slice bounds:              {:.2} ns/op (result: {})",
        ns_per_op,
        black_box(result)
    );

    println!();
    println!("Bounds check overhead: {:.2} ns/op",
        {
            let safe_start = Instant::now();
            black_box(benchmark_array_access_safe(&data));
            let safe_time = safe_start.elapsed();

            let unchecked_start = Instant::now();
            black_box(benchmark_array_access_unchecked(&data));
            let unchecked_time = unchecked_start.elapsed();

            (safe_time.as_nanos() as f64 - unchecked_time.as_nanos() as f64)
                / (ITERATIONS * ARRAY_SIZE) as f64
        }
    );
}
