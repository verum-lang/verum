//! Production Target Benchmarks for CBGR
//!
//! Target: CBGR check < 15ns
//!
//! This benchmarks the core CBGR operations that correspond to the runtime
//! check path: generation comparison, epoch validation, and tier analysis.
//! The actual runtime check is emitted as inline LLVM IR (load gen, cmp, branch),
//! so we measure the equivalent operations here.

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use std::sync::atomic::{AtomicU32, Ordering};
use verum_cbgr::{CbgrTier, ReferenceTier, Tier0Reason, TierStatistics};

// ============================================================================
// Simulated CBGR Runtime Check (mirrors LLVM IR codegen path)
// ============================================================================

/// Simulates the ThinRef layout: ptr(8) + generation(4) + epoch_caps(4) = 16 bytes
#[repr(C)]
struct ThinRef {
    ptr: *const u8,
    generation: u32,
    epoch_caps: u32,
}

/// Simulates the allocation header that stores the current generation
struct AllocationSlot {
    generation: AtomicU32,
    epoch: AtomicU32,
}

/// The core CBGR check as it would be emitted in LLVM IR:
/// 1. Load ref.generation
/// 2. Load slot.generation (atomic acquire)
/// 3. Compare generations
/// 4. Branch on mismatch -> panic path
///
/// This is the hot path that must be < 15ns.
#[inline(never)]
fn cbgr_check_inline(thin_ref: &ThinRef, slot: &AllocationSlot) -> bool {
    let ref_gen = thin_ref.generation;
    let slot_gen = slot.generation.load(Ordering::Acquire);
    ref_gen == slot_gen
}

/// Extended check including epoch validation
#[inline(never)]
fn cbgr_check_with_epoch(thin_ref: &ThinRef, slot: &AllocationSlot) -> bool {
    let ref_gen = thin_ref.generation;
    let slot_gen = slot.generation.load(Ordering::Acquire);
    if ref_gen != slot_gen {
        return false;
    }
    let ref_epoch = thin_ref.epoch_caps >> 16;
    let slot_epoch = slot.epoch.load(Ordering::Acquire);
    slot_epoch.wrapping_sub(ref_epoch) < 0x1000_0000
}

// ============================================================================
// Benchmarks
// ============================================================================

fn bench_cbgr_check_target(c: &mut Criterion) {
    let mut group = c.benchmark_group("production_target_cbgr_check");

    let data: Vec<u8> = vec![42; 64];
    let slot = AllocationSlot {
        generation: AtomicU32::new(1),
        epoch: AtomicU32::new(0),
    };

    let thin_ref = ThinRef {
        ptr: data.as_ptr(),
        generation: 1,
        epoch_caps: 0,
    };

    // TARGET: < 15ns for a single generation check
    group.bench_function("generation_check_valid", |b| {
        b.iter(|| black_box(cbgr_check_inline(black_box(&thin_ref), black_box(&slot))))
    });

    // Check with epoch validation
    group.bench_function("generation_and_epoch_check", |b| {
        b.iter(|| {
            black_box(cbgr_check_with_epoch(
                black_box(&thin_ref),
                black_box(&slot),
            ))
        })
    });

    // Invalid reference (generation mismatch) - should still be fast
    let stale_ref = ThinRef {
        ptr: data.as_ptr(),
        generation: 0, // stale
        epoch_caps: 0,
    };
    group.bench_function("generation_check_invalid", |b| {
        b.iter(|| black_box(cbgr_check_inline(black_box(&stale_ref), black_box(&slot))))
    });

    // Batch of 100 checks (amortized cost)
    let refs: Vec<ThinRef> = (0..100)
        .map(|_| ThinRef {
            ptr: data.as_ptr(),
            generation: 1,
            epoch_caps: 0,
        })
        .collect();
    group.bench_function("100_checks_batch", |b| {
        b.iter(|| {
            let mut valid = 0u32;
            for r in &refs {
                if cbgr_check_inline(r, &slot) {
                    valid += 1;
                }
            }
            black_box(valid)
        })
    });

    group.finish();
}

fn bench_tier_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("cbgr_tier_operations");

    group.bench_function("tier_creation_tier0", |b| {
        b.iter(|| black_box(ReferenceTier::tier0(Tier0Reason::NotAnalyzed)))
    });

    group.bench_function("tier_creation_tier1", |b| {
        b.iter(|| black_box(ReferenceTier::tier1()))
    });

    group.bench_function("tier_number_lookup", |b| {
        let tier = ReferenceTier::tier0(Tier0Reason::NotAnalyzed);
        b.iter(|| black_box(tier.tier_number()))
    });

    group.bench_function("tier_to_vbc", |b| {
        let tier = ReferenceTier::tier1();
        b.iter(|| black_box(tier.to_vbc_tier()))
    });

    // Statistics recording (common in analysis)
    group.bench_function("statistics_record_1000", |b| {
        let tiers = [
            ReferenceTier::tier0(Tier0Reason::NotAnalyzed),
            ReferenceTier::tier1(),
            ReferenceTier::tier2(),
        ];
        b.iter(|| {
            let mut stats = TierStatistics::default();
            for i in 0..1000 {
                stats.record(&tiers[i % 3]);
            }
            black_box(stats)
        })
    });

    group.finish();
}

fn bench_cbgr_tier_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("cbgr_tier_comparison");

    group.bench_function("tier_eq_check", |b| {
        let t1 = CbgrTier::Tier0;
        let t2 = CbgrTier::Tier1;
        b.iter(|| black_box(black_box(t1) == black_box(t2)))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_cbgr_check_target,
    bench_tier_operations,
    bench_cbgr_tier_comparison,
);
criterion_main!(benches);
