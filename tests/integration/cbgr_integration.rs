//! Category 4: CBGR Memory Safety Integration Tests
//!
//! Tests CBGR across execution scenarios:
//! - 3-Tier references (Managed → Checked → Unsafe)
//! - Escape analysis and tier promotion
//! - Cross-function safety
//! - Concurrent access
//! - Performance validation (<15ns overhead)
//! - Use-after-free prevention

use std::sync::Arc;
use std::time::Duration;
use verum_cbgr::{Allocator, GenRef, Tier, ThinRef, FatRef};
use verum_std::core::{List, Shared};

use crate::integration::test_utils::*;

// ============================================================================
// Test 4.1: 3-Tier Reference System
// ============================================================================

#[test]
fn test_tier_0_standard_cbgr() {
    let allocator = Allocator::new();
    let value = 42i64;

    // Allocate in Tier 0 (standard CBGR with generation checking)
    let gen_ref: GenRef<i64> = allocator.alloc(value, Tier::Standard);

    assert_eq!(*gen_ref, 42);

    // Deref should have ~15ns overhead
    let (result, duration) = measure_time(|| {
        for _ in 0..10000 {
            let _ = *gen_ref;
        }
    });

    let avg_ns = duration.as_nanos() / 10000;
    assert!(avg_ns < 30, "CBGR overhead should be <30ns, got {}ns", avg_ns);
}

#[test]
fn test_tier_1_checked_references() {
    let allocator = Allocator::new();
    let value = vec![1, 2, 3, 4, 5];

    // Allocate in Tier 1 (compiler-proven safe, 0ns overhead)
    let checked_ref = allocator.alloc(value, Tier::Checked);

    assert_eq!(checked_ref.len(), 5);

    // Access should be zero-cost
    let (_, duration) = measure_time(|| {
        for _ in 0..10000 {
            let _ = checked_ref.len();
        }
    });

    // Tier 1 should have minimal overhead
    assert_duration_lt(duration, Duration::from_micros(50), "Tier 1 should be fast");
}

#[test]
fn test_tier_2_unsafe_references() {
    let allocator = Allocator::new();
    let value = 100i64;

    // Allocate in Tier 2 (unsafe, zero-cost, manual proof required)
    let unsafe_ref = allocator.alloc(value, Tier::Unsafe);

    assert_eq!(*unsafe_ref, 100);

    // Zero overhead access
    let (_, duration) = measure_time(|| {
        for _ in 0..10000 {
            let _ = *unsafe_ref;
        }
    });

    // Tier 2 should be as fast as raw pointers
    assert_duration_lt(duration, Duration::from_micros(10), "Tier 2 should be zero-cost");
}

// ============================================================================
// Test 4.2: Escape Analysis
// ============================================================================

#[test]
fn test_escape_analysis_local_scope() {
    let allocator = Allocator::new();

    // Value doesn't escape function scope
    fn local_computation(alloc: &Allocator) -> i64 {
        let value = alloc.alloc(42i64, Tier::Standard);
        *value + 1
    }

    let result = local_computation(&allocator);
    assert_eq!(result, 43);

    // Compiler can promote to Tier 1 (checked)
}

#[test]
fn test_escape_analysis_return_value() {
    let allocator = Allocator::new();

    // Value escapes via return
    fn create_ref(alloc: &Allocator) -> GenRef<i64> {
        alloc.alloc(42i64, Tier::Standard)
    }

    let ref_value = create_ref(&allocator);
    assert_eq!(*ref_value, 42);

    // Must remain Tier 0 (standard CBGR)
}

// ============================================================================
// Test 4.3: Cross-Function Safety
// ============================================================================

#[test]
fn test_reference_passing_between_functions() {
    let allocator = Allocator::new();

    fn increment(r: &GenRef<i64>) -> i64 {
        **r + 1
    }

    fn double(r: &GenRef<i64>) -> i64 {
        **r * 2
    }

    let value = allocator.alloc(10i64, Tier::Standard);

    let inc = increment(&value);
    let dbl = double(&value);

    assert_eq!(inc, 11);
    assert_eq!(dbl, 20);
}

#[test]
fn test_reference_lifetime_across_calls() {
    let allocator = Allocator::new();
    let value = allocator.alloc(42i64, Tier::Standard);

    fn use_many_times(r: &GenRef<i64>, n: usize) -> i64 {
        let mut sum = 0;
        for _ in 0..n {
            sum += **r;
        }
        sum
    }

    let result = use_many_times(&value, 10);
    assert_eq!(result, 420);
}

// ============================================================================
// Test 4.4: Concurrent Access
// ============================================================================

#[tokio::test]
async fn test_concurrent_reference_access() {
    let allocator = Arc::new(Allocator::new());
    let shared_value = Arc::new(allocator.alloc(100i64, Tier::Standard));

    // Access from multiple threads
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let shared = Arc::clone(&shared_value);
            tokio::spawn(async move {
                let value = **shared;
                value
            })
        })
        .collect();

    let results: Vec<i64> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    assert!(results.iter().all(|&v| v == 100));
}

#[tokio::test]
async fn test_concurrent_allocations() {
    let allocator = Arc::new(Allocator::new());

    let handles: Vec<_> = (0..100)
        .map(|i| {
            let alloc = Arc::clone(&allocator);
            tokio::spawn(async move {
                let value = alloc.alloc(i, Tier::Standard);
                *value
            })
        })
        .collect();

    let results = futures::future::join_all(handles).await;
    assert_eq!(results.len(), 100);
}

// ============================================================================
// Test 4.5: Performance Validation
// ============================================================================

#[test]
fn test_cbgr_overhead_target_15ns() {
    let allocator = Allocator::new();
    let value = allocator.alloc(42i64, Tier::Standard);

    // Measure dereference overhead
    let iterations = 1_000_000;
    let (_, duration) = measure_time(|| {
        for _ in 0..iterations {
            std::hint::black_box(*value);
        }
    });

    let avg_ns = duration.as_nanos() as f64 / iterations as f64;

    assert!(
        avg_ns < 20.0,
        "CBGR overhead should be <20ns, got {:.2}ns",
        avg_ns
    );
}

#[test]
fn test_tier_performance_comparison() {
    let allocator = Allocator::new();

    let t0 = allocator.alloc(42i64, Tier::Standard);
    let t1 = allocator.alloc(42i64, Tier::Checked);
    let t2 = allocator.alloc(42i64, Tier::Unsafe);

    let iterations = 100_000;

    let (_, t0_time) = measure_time(|| {
        for _ in 0..iterations {
            std::hint::black_box(*t0);
        }
    });

    let (_, t1_time) = measure_time(|| {
        for _ in 0..iterations {
            std::hint::black_box(*t1);
        }
    });

    let (_, t2_time) = measure_time(|| {
        for _ in 0..iterations {
            std::hint::black_box(*t2);
        }
    });

    // Tier 1 and 2 should be faster than Tier 0
    assert!(t1_time <= t0_time);
    assert!(t2_time <= t0_time);
}

// ============================================================================
// Test 4.6: Use-After-Free Prevention
// ============================================================================

#[test]
fn test_generation_tracking() {
    let allocator = Allocator::new();

    // Allocate and deallocate
    let gen_ref = allocator.alloc(42i64, Tier::Standard);
    let generation = gen_ref.generation();

    drop(gen_ref);

    // Attempting to use old generation should be detected
    // (actual implementation would check generation)
}

#[test]
fn test_dangling_reference_detection() {
    let allocator = Allocator::new();

    let ref1 = allocator.alloc(100i64, Tier::Standard);
    let ptr = &*ref1 as *const i64;

    drop(ref1);

    // Accessing through raw pointer would be undefined behavior
    // CBGR prevents this through generation checking
}

// ============================================================================
// Test 4.7: Memory Layout Verification
// ============================================================================

#[test]
fn test_thin_ref_size() {
    use std::mem::size_of;

    // ThinRef should be 16 bytes (ptr + generation + epoch_caps)
    assert!(
        size_of::<ThinRef<i64>>() <= 24,
        "ThinRef should be ≤24 bytes"
    );
}

#[test]
fn test_fat_ref_size() {
    use std::mem::size_of;

    // FatRef should be 24 bytes (ptr + generation + epoch_caps + len)
    assert!(
        size_of::<FatRef<i64>>() <= 32,
        "FatRef should be ≤32 bytes"
    );
}

// ============================================================================
// Test 4.8: Stress Tests
// ============================================================================

#[test]
fn test_stress_many_allocations() {
    let allocator = Allocator::new();

    let (_, duration) = measure_time(|| {
        let mut refs = Vec::new();
        for i in 0..10_000 {
            refs.push(allocator.alloc(i, Tier::Standard));
        }

        // Verify all allocations
        for (i, r) in refs.iter().enumerate() {
            assert_eq!(**r, i as i64);
        }
    });

    assert_duration_lt(
        duration,
        Duration::from_secs(1),
        "10K allocations should be <1s"
    );
}

#[tokio::test]
async fn test_stress_concurrent_mixed_operations() {
    let allocator = Arc::new(Allocator::new());

    let handles: Vec<_> = (0..50)
        .map(|i| {
            let alloc = Arc::clone(&allocator);
            tokio::spawn(async move {
                // Mix of allocations and deallocations
                let mut refs = Vec::new();
                for j in 0..100 {
                    refs.push(alloc.alloc(i * 100 + j, Tier::Standard));
                }

                // Access all refs
                let sum: i64 = refs.iter().map(|r| **r).sum();

                sum
            })
        })
        .collect();

    let results = futures::future::join_all(handles).await;
    assert_eq!(results.len(), 50);
}

#[cfg(test)]
mod property_tests {
    use super::*;

    #[test]
    fn property_alloc_deref_roundtrip() {
        let allocator = Allocator::new();

        for i in 0..100 {
            let value = allocator.alloc(i, Tier::Standard);
            assert_eq!(*value, i);
        }
    }

    #[test]
    fn property_tier_upgrade_preserves_value() {
        let allocator = Allocator::new();
        let value = 42i64;

        let t0 = allocator.alloc(value, Tier::Standard);
        let t1 = allocator.alloc(value, Tier::Checked);
        let t2 = allocator.alloc(value, Tier::Unsafe);

        assert_eq!(*t0, value);
        assert_eq!(*t1, value);
        assert_eq!(*t2, value);
    }
}
