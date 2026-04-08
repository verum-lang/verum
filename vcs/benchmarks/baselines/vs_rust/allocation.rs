//! Allocation Baseline - Rust Implementation
//!
//! Measures allocation performance in Rust as a baseline for Verum.
//!
//! Compile: rustc -O -o allocation allocation.rs

use std::hint::black_box;
use std::time::Instant;

const SMALL_ITERATIONS: usize = 1_000_000;
const MEDIUM_ITERATIONS: usize = 100_000;
const LARGE_ITERATIONS: usize = 1_000;

#[derive(Clone)]
struct SmallStruct {
    a: i32,
    b: i32,
    c: f64,
    d: bool,
}

fn benchmark_small_alloc() {
    for _ in 0..SMALL_ITERATIONS {
        let obj = Box::new(SmallStruct {
            a: 1,
            b: 2,
            c: 3.14,
            d: true,
        });
        black_box(obj);
    }
}

fn benchmark_medium_alloc() {
    for _ in 0..MEDIUM_ITERATIONS {
        let data: Box<[u8; 1024]> = Box::new([0u8; 1024]);
        black_box(data);
    }
}

fn benchmark_large_alloc() {
    for _ in 0..LARGE_ITERATIONS {
        let data: Vec<u8> = vec![0u8; 1024 * 1024];
        black_box(data);
    }
}

fn benchmark_vec_growth() {
    for _ in 0..1000 {
        let mut v: Vec<i32> = Vec::new();
        for i in 0..10_000 {
            v.push(i);
        }
        black_box(v);
    }
}

fn benchmark_vec_with_capacity() {
    for _ in 0..1000 {
        let mut v: Vec<i32> = Vec::with_capacity(10_000);
        for i in 0..10_000 {
            v.push(i);
        }
        black_box(v);
    }
}

fn benchmark_rc_alloc() {
    use std::rc::Rc;

    for _ in 0..SMALL_ITERATIONS {
        let rc = Rc::new(SmallStruct {
            a: 1,
            b: 2,
            c: 3.14,
            d: true,
        });
        let _clone1 = Rc::clone(&rc);
        let _clone2 = Rc::clone(&rc);
        black_box(rc);
    }
}

fn benchmark_arc_alloc() {
    use std::sync::Arc;

    for _ in 0..SMALL_ITERATIONS {
        let arc = Arc::new(SmallStruct {
            a: 1,
            b: 2,
            c: 3.14,
            d: true,
        });
        let _clone1 = Arc::clone(&arc);
        let _clone2 = Arc::clone(&arc);
        black_box(arc);
    }
}

fn main() {
    // Warmup
    benchmark_small_alloc();

    // Small allocation
    let start = Instant::now();
    benchmark_small_alloc();
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / SMALL_ITERATIONS as f64;
    println!("Small alloc (Box):         {:.2} ns/op", ns_per_op);

    // Medium allocation
    let start = Instant::now();
    benchmark_medium_alloc();
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / MEDIUM_ITERATIONS as f64;
    println!("Medium alloc (1KB Box):    {:.2} ns/op", ns_per_op);

    // Large allocation
    let start = Instant::now();
    benchmark_large_alloc();
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / LARGE_ITERATIONS as f64;
    println!("Large alloc (1MB Vec):     {:.2} ns/op", ns_per_op);

    // Vec growth
    let start = Instant::now();
    benchmark_vec_growth();
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / (1000 * 10_000) as f64;
    println!("Vec growth (push):         {:.2} ns/op", ns_per_op);

    // Vec with capacity
    let start = Instant::now();
    benchmark_vec_with_capacity();
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / (1000 * 10_000) as f64;
    println!("Vec with_capacity:         {:.2} ns/op", ns_per_op);

    // Rc allocation
    let start = Instant::now();
    benchmark_rc_alloc();
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / SMALL_ITERATIONS as f64;
    println!("Rc alloc + 2 clones:       {:.2} ns/op", ns_per_op);

    // Arc allocation
    let start = Instant::now();
    benchmark_arc_alloc();
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / SMALL_ITERATIONS as f64;
    println!("Arc alloc + 2 clones:      {:.2} ns/op", ns_per_op);
}
