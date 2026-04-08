//! Function Call Overhead Baseline - Rust Implementation
//!
//! This benchmark measures function call overhead in Rust as a comparison
//! for Verum's function call performance targets.
//!
//! Compile: rustc -O -o function_call function_call.rs

use std::hint::black_box;
use std::time::Instant;

const WARMUP_ITERATIONS: usize = 100_000;
const BENCHMARK_ITERATIONS: usize = 10_000_000;

#[inline(never)]
fn direct_add(a: i64, b: i64) -> i64 {
    a + b
}

#[inline(never)]
fn complex_function(a: i64, b: i64, c: i64) -> i64 {
    let x = a * b;
    let y = b * c;
    let z = c * a;
    x + y + z
}

// Trait for virtual dispatch
trait Computable {
    fn compute(&self, arg: i64) -> i64;
}

struct Impl1 { value: i64 }
struct Impl2 { value: i64 }
struct Impl3 { value: i64 }

impl Computable for Impl1 {
    #[inline(never)]
    fn compute(&self, arg: i64) -> i64 { self.value + arg }
}

impl Computable for Impl2 {
    #[inline(never)]
    fn compute(&self, arg: i64) -> i64 { self.value * arg }
}

impl Computable for Impl3 {
    #[inline(never)]
    fn compute(&self, arg: i64) -> i64 { self.value - arg }
}

fn benchmark_inline() {
    let mut sum: i64 = 0;

    for _ in 0..WARMUP_ITERATIONS {
        sum = black_box(sum + 1);
    }

    sum = 0;
    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        sum = black_box(sum + 1);
    }

    let elapsed = start.elapsed();
    black_box(sum);

    let ns_per_op = elapsed.as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    println!("inline_baseline:      {:.2} ns/op", ns_per_op);
}

fn benchmark_direct_call() {
    let mut sum: i64 = 0;

    for _ in 0..WARMUP_ITERATIONS {
        sum = direct_add(sum, 1);
    }

    sum = 0;
    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        sum = direct_add(sum, 1);
    }

    let elapsed = start.elapsed();
    black_box(sum);

    let ns_per_op = elapsed.as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    println!("direct_call:          {:.2} ns/op", ns_per_op);
}

fn benchmark_complex_call() {
    let mut result: i64 = 0;

    for i in 0..WARMUP_ITERATIONS as i64 {
        result = complex_function(result, i, i + 1);
    }

    result = 0;
    let start = Instant::now();

    for i in 0..BENCHMARK_ITERATIONS as i64 {
        result = complex_function(result, i, i + 1);
    }

    let elapsed = start.elapsed();
    black_box(result);

    let ns_per_op = elapsed.as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    println!("complex_call:         {:.2} ns/op", ns_per_op);
}

fn benchmark_virtual_call_monomorphic() {
    let obj = Impl1 { value: 0 };
    let dyn_obj: &dyn Computable = &obj;
    let mut sum: i64 = 0;

    for _ in 0..WARMUP_ITERATIONS {
        sum = dyn_obj.compute(1);
    }

    sum = 0;
    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        sum = dyn_obj.compute(1);
    }

    let elapsed = start.elapsed();
    black_box(sum);

    let ns_per_op = elapsed.as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    println!("virtual_monomorphic:  {:.2} ns/op", ns_per_op);
}

fn benchmark_virtual_call_polymorphic() {
    let objs: [&dyn Computable; 3] = [
        &Impl1 { value: 1 },
        &Impl2 { value: 2 },
        &Impl3 { value: 3 },
    ];
    let mut sum: i64 = 0;

    for i in 0..WARMUP_ITERATIONS {
        sum = sum.wrapping_add(objs[i % 3].compute(1));
    }

    sum = 0;
    let start = Instant::now();

    for i in 0..BENCHMARK_ITERATIONS {
        sum = sum.wrapping_add(objs[i % 3].compute(1));
    }

    let elapsed = start.elapsed();
    black_box(sum);

    let ns_per_op = elapsed.as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    println!("virtual_polymorphic:  {:.2} ns/op", ns_per_op);
}

fn benchmark_closure_no_capture() {
    let closure = |a: i64, b: i64| -> i64 { a + b };
    let mut sum: i64 = 0;

    for _ in 0..WARMUP_ITERATIONS {
        sum = closure(sum, 1);
    }

    sum = 0;
    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        sum = closure(sum, 1);
    }

    let elapsed = start.elapsed();
    black_box(sum);

    let ns_per_op = elapsed.as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    println!("closure_no_capture:   {:.2} ns/op", ns_per_op);
}

fn benchmark_closure_with_capture() {
    let multiplier: i64 = 2;
    let offset: i64 = 10;
    let closure = |a: i64| -> i64 { a * multiplier + offset };
    let mut sum: i64 = 0;

    for i in 0..WARMUP_ITERATIONS as i64 {
        sum = closure(i);
    }

    sum = 0;
    let start = Instant::now();

    for i in 0..BENCHMARK_ITERATIONS as i64 {
        sum = closure(i);
    }

    let elapsed = start.elapsed();
    black_box(sum);

    let ns_per_op = elapsed.as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    println!("closure_with_capture: {:.2} ns/op", ns_per_op);
}

fn benchmark_closure_dyn_fn() {
    let closure: &dyn Fn(i64, i64) -> i64 = &|a, b| a + b;
    let mut sum: i64 = 0;

    for _ in 0..WARMUP_ITERATIONS {
        sum = closure(sum, 1);
    }

    sum = 0;
    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        sum = closure(sum, 1);
    }

    let elapsed = start.elapsed();
    black_box(sum);

    let ns_per_op = elapsed.as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    println!("closure_dyn_fn:       {:.2} ns/op", ns_per_op);
}

fn benchmark_generic_call() {
    #[inline(never)]
    fn generic_add<T: std::ops::Add<Output = T>>(a: T, b: T) -> T {
        a + b
    }

    let mut sum: i64 = 0;

    for _ in 0..WARMUP_ITERATIONS {
        sum = generic_add(sum, 1i64);
    }

    sum = 0;
    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        sum = generic_add(sum, 1i64);
    }

    let elapsed = start.elapsed();
    black_box(sum);

    let ns_per_op = elapsed.as_nanos() as f64 / BENCHMARK_ITERATIONS as f64;
    println!("generic_call:         {:.2} ns/op", ns_per_op);
}

#[inline(never)]
fn recursive_sum(n: i64) -> i64 {
    if n <= 0 { 0 } else { n + recursive_sum(n - 1) }
}

fn benchmark_recursive_call() {
    let depth = 10;
    let iterations = BENCHMARK_ITERATIONS / depth;

    for _ in 0..(WARMUP_ITERATIONS / depth) {
        black_box(recursive_sum(depth as i64));
    }

    let start = Instant::now();

    for _ in 0..iterations {
        black_box(recursive_sum(depth as i64));
    }

    let elapsed = start.elapsed();

    let ns_per_op = elapsed.as_nanos() as f64 / (iterations * depth) as f64;
    println!("recursive_depth_10:   {:.2} ns/op", ns_per_op);
}

#[inline(never)]
fn tail_sum(n: i64, acc: i64) -> i64 {
    if n <= 0 { acc } else { tail_sum(n - 1, acc + n) }
}

fn benchmark_tail_recursive() {
    let depth = 100;
    let iterations = BENCHMARK_ITERATIONS / depth;

    for _ in 0..(WARMUP_ITERATIONS / depth) {
        black_box(tail_sum(depth as i64, 0));
    }

    let start = Instant::now();

    for _ in 0..iterations {
        black_box(tail_sum(depth as i64, 0));
    }

    let elapsed = start.elapsed();

    let ns_per_op = elapsed.as_nanos() as f64 / (iterations * depth) as f64;
    println!("tail_recursive_100:   {:.2} ns/op", ns_per_op);
}

fn main() {
    println!("=== Rust Function Call Baseline ===\n");

    benchmark_inline();
    benchmark_direct_call();
    benchmark_complex_call();
    benchmark_virtual_call_monomorphic();
    benchmark_virtual_call_polymorphic();
    benchmark_closure_no_capture();
    benchmark_closure_with_capture();
    benchmark_closure_dyn_fn();
    benchmark_generic_call();
    benchmark_recursive_call();
    benchmark_tail_recursive();

    println!();
    println!("Verum targets (vs Rust):");
    println!("  - Direct call:  Equivalent to Rust (< 5ns)");
    println!("  - Virtual call: Equivalent to Rust dyn (< 15ns)");
    println!("  - Closure:      Equivalent to Rust Fn (< 10ns)");
    println!("  - Generic:      Equivalent (monomorphized)");
}
