//! SIMD Throughput Baseline - Rust Implementation
//!
//! This benchmark measures SIMD performance in Rust as a comparison
//! for Verum's SIMD performance targets.
//!
//! Compile: RUSTFLAGS="-C target-feature=+avx2,+fma" rustc -O -o simd simd.rs

use std::hint::black_box;
use std::time::Instant;

const WARMUP_ITERATIONS: usize = 1000;
const BENCHMARK_ITERATIONS: usize = 10000;
const ARRAY_SIZE: usize = 1024 * 1024;

fn benchmark_scalar_add() {
    let mut a: Vec<f32> = (0..ARRAY_SIZE).map(|i| i as f32).collect();
    let b: Vec<f32> = (0..ARRAY_SIZE).map(|i| (i * 2) as f32).collect();

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        for i in 0..ARRAY_SIZE {
            a[i] = a[i] + b[i];
        }
    }

    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        for i in 0..ARRAY_SIZE {
            a[i] = a[i] + b[i];
        }
    }

    let elapsed = start.elapsed();
    black_box(&a);

    let ops = BENCHMARK_ITERATIONS as f64 * ARRAY_SIZE as f64;
    let gflops = ops / elapsed.as_secs_f64() / 1e9;
    println!("scalar_add_f32:       {:.2} GFLOPS", gflops);
}

fn benchmark_autovec_add() {
    let mut a: Vec<f32> = (0..ARRAY_SIZE).map(|i| i as f32).collect();
    let b: Vec<f32> = (0..ARRAY_SIZE).map(|i| (i * 2) as f32).collect();

    // Warmup - should auto-vectorize
    for _ in 0..WARMUP_ITERATIONS {
        a.iter_mut().zip(b.iter()).for_each(|(a, b)| *a += *b);
    }

    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        a.iter_mut().zip(b.iter()).for_each(|(a, b)| *a += *b);
    }

    let elapsed = start.elapsed();
    black_box(&a);

    let ops = BENCHMARK_ITERATIONS as f64 * ARRAY_SIZE as f64;
    let gflops = ops / elapsed.as_secs_f64() / 1e9;
    println!("autovec_add_f32:      {:.2} GFLOPS", gflops);
}

#[cfg(target_arch = "x86_64")]
fn benchmark_explicit_simd_add() {
    use std::arch::x86_64::*;

    let mut a: Vec<f32> = (0..ARRAY_SIZE).map(|i| i as f32).collect();
    let b: Vec<f32> = (0..ARRAY_SIZE).map(|i| (i * 2) as f32).collect();

    let vec_count = ARRAY_SIZE / 8;

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        unsafe {
            let a_ptr = a.as_mut_ptr();
            let b_ptr = b.as_ptr();
            for i in 0..vec_count {
                let offset = i * 8;
                let va = _mm256_loadu_ps(a_ptr.add(offset));
                let vb = _mm256_loadu_ps(b_ptr.add(offset));
                let result = _mm256_add_ps(va, vb);
                _mm256_storeu_ps(a_ptr.add(offset), result);
            }
        }
    }

    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        unsafe {
            let a_ptr = a.as_mut_ptr();
            let b_ptr = b.as_ptr();
            for i in 0..vec_count {
                let offset = i * 8;
                let va = _mm256_loadu_ps(a_ptr.add(offset));
                let vb = _mm256_loadu_ps(b_ptr.add(offset));
                let result = _mm256_add_ps(va, vb);
                _mm256_storeu_ps(a_ptr.add(offset), result);
            }
        }
    }

    let elapsed = start.elapsed();
    black_box(&a);

    let ops = BENCHMARK_ITERATIONS as f64 * ARRAY_SIZE as f64;
    let gflops = ops / elapsed.as_secs_f64() / 1e9;
    println!("avx2_add_f32x8:       {:.2} GFLOPS", gflops);
}

#[cfg(target_arch = "x86_64")]
fn benchmark_fma() {
    use std::arch::x86_64::*;

    let mut a: Vec<f32> = (0..ARRAY_SIZE).map(|i| (i % 1000) as f32 * 0.001).collect();
    let b: Vec<f32> = vec![2.0f32; ARRAY_SIZE];
    let c: Vec<f32> = vec![1.0f32; ARRAY_SIZE];

    let vec_count = ARRAY_SIZE / 8;

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        unsafe {
            let a_ptr = a.as_mut_ptr();
            let b_ptr = b.as_ptr();
            let c_ptr = c.as_ptr();
            for i in 0..vec_count {
                let offset = i * 8;
                let va = _mm256_loadu_ps(a_ptr.add(offset));
                let vb = _mm256_loadu_ps(b_ptr.add(offset));
                let vc = _mm256_loadu_ps(c_ptr.add(offset));
                let result = _mm256_fmadd_ps(va, vb, vc);
                _mm256_storeu_ps(a_ptr.add(offset), result);
            }
        }
    }

    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        unsafe {
            let a_ptr = a.as_mut_ptr();
            let b_ptr = b.as_ptr();
            let c_ptr = c.as_ptr();
            for i in 0..vec_count {
                let offset = i * 8;
                let va = _mm256_loadu_ps(a_ptr.add(offset));
                let vb = _mm256_loadu_ps(b_ptr.add(offset));
                let vc = _mm256_loadu_ps(c_ptr.add(offset));
                let result = _mm256_fmadd_ps(va, vb, vc);
                _mm256_storeu_ps(a_ptr.add(offset), result);
            }
        }
    }

    let elapsed = start.elapsed();
    black_box(&a);

    // FMA counts as 2 operations
    let ops = BENCHMARK_ITERATIONS as f64 * ARRAY_SIZE as f64 * 2.0;
    let gflops = ops / elapsed.as_secs_f64() / 1e9;
    println!("avx2_fma_f32x8:       {:.2} GFLOPS", gflops);
}

#[cfg(target_arch = "x86_64")]
fn benchmark_dot_product() {
    use std::arch::x86_64::*;

    let a: Vec<f32> = (0..ARRAY_SIZE).map(|i| (i % 100) as f32 * 0.01).collect();
    let b: Vec<f32> = (0..ARRAY_SIZE).map(|i| (i % 100) as f32 * 0.02).collect();

    let vec_count = ARRAY_SIZE / 8;
    let mut result: f32 = 0.0;

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        unsafe {
            let mut sum = _mm256_setzero_ps();
            let a_ptr = a.as_ptr();
            let b_ptr = b.as_ptr();
            for i in 0..vec_count {
                let offset = i * 8;
                let va = _mm256_loadu_ps(a_ptr.add(offset));
                let vb = _mm256_loadu_ps(b_ptr.add(offset));
                sum = _mm256_fmadd_ps(va, vb, sum);
            }
            // Horizontal sum
            let hi = _mm256_extractf128_ps(sum, 1);
            let lo = _mm256_castps256_ps128(sum);
            let sum128 = _mm_add_ps(lo, hi);
            let sum128 = _mm_hadd_ps(sum128, sum128);
            let sum128 = _mm_hadd_ps(sum128, sum128);
            result = _mm_cvtss_f32(sum128);
        }
    }

    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        unsafe {
            let mut sum = _mm256_setzero_ps();
            let a_ptr = a.as_ptr();
            let b_ptr = b.as_ptr();
            for i in 0..vec_count {
                let offset = i * 8;
                let va = _mm256_loadu_ps(a_ptr.add(offset));
                let vb = _mm256_loadu_ps(b_ptr.add(offset));
                sum = _mm256_fmadd_ps(va, vb, sum);
            }
            let hi = _mm256_extractf128_ps(sum, 1);
            let lo = _mm256_castps256_ps128(sum);
            let sum128 = _mm_add_ps(lo, hi);
            let sum128 = _mm_hadd_ps(sum128, sum128);
            let sum128 = _mm_hadd_ps(sum128, sum128);
            result = _mm_cvtss_f32(sum128);
        }
    }

    let elapsed = start.elapsed();
    black_box(result);

    let ops = BENCHMARK_ITERATIONS as f64 * ARRAY_SIZE as f64 * 2.0;
    let gflops = ops / elapsed.as_secs_f64() / 1e9;
    println!("avx2_dot_product:     {:.2} GFLOPS", gflops);
}

fn benchmark_memory_bandwidth() {
    let src: Vec<f32> = (0..ARRAY_SIZE).map(|i| i as f32).collect();
    let mut dst: Vec<f32> = vec![0.0f32; ARRAY_SIZE];

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        dst.copy_from_slice(&src);
    }

    let start = Instant::now();

    for _ in 0..BENCHMARK_ITERATIONS {
        dst.copy_from_slice(&src);
    }

    let elapsed = start.elapsed();
    black_box(&dst);

    let bytes = BENCHMARK_ITERATIONS as f64 * ARRAY_SIZE as f64 * 4.0 * 2.0; // read + write
    let gb_per_sec = bytes / elapsed.as_secs_f64() / 1e9;
    println!("memory_bandwidth:     {:.2} GB/s", gb_per_sec);
}

fn main() {
    println!("=== Rust SIMD Throughput Baseline ===\n");

    benchmark_scalar_add();
    benchmark_autovec_add();

    #[cfg(target_arch = "x86_64")]
    {
        benchmark_explicit_simd_add();
        benchmark_fma();
        benchmark_dot_product();
    }

    benchmark_memory_bandwidth();

    println!();
    println!("Verum SIMD targets (vs Rust):");
    println!("  - Explicit SIMD: Equivalent to Rust intrinsics");
    println!("  - Auto-vectorization: > 70% of explicit SIMD");
    println!("  - Overall: > 80% of Rust SIMD performance");
}
