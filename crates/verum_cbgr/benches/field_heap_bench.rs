//! Benchmarks for Field-Sensitive Heap Tracking
//!
//! Benchmarks for CBGR field-sensitive heap tracking. Tracks which struct
//! fields are stored to heap vs stack for independent per-field CBGR tier
//! decisions. Validates O(fields * heap_stores) performance target and
//! <100ms for 10K LOC analysis.
//!
//! **Benchmark Coverage**: 5+ comprehensive scenarios

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use verum_cbgr::analysis::{FieldPath, RefId};
use verum_cbgr::field_heap_tracking::{FieldHeapTracker, HeapSiteId};
use verum_common::{Set, Text};

// ==================================================================================
// Benchmark 1: Field Heap Tracker Creation
// ==================================================================================

fn bench_tracker_creation(c: &mut Criterion) {
    c.bench_function("tracker_creation", |b| {
        b.iter(|| {
            let tracker = FieldHeapTracker::new();
            black_box(tracker);
        });
    });
}

// ==================================================================================
// Benchmark 2: Heap Allocation Registration
// ==================================================================================

fn bench_heap_allocation_registration(c: &mut Criterion) {
    let mut group = c.benchmark_group("heap_allocation_registration");

    for count in [10, 50, 100, 500].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            b.iter(|| {
                let mut tracker = FieldHeapTracker::new();
                for i in 0..count {
                    let site = tracker.register_heap_allocation(format!("heap_{}", i));
                    black_box(site);
                }
            });
        });
    }

    group.finish();
}

// ==================================================================================
// Benchmark 3: Heap Store Addition
// ==================================================================================

fn bench_heap_store_addition(c: &mut Criterion) {
    let mut group = c.benchmark_group("heap_store_addition");

    for stores in [10, 50, 100, 500, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(stores), stores, |b, &stores| {
            b.iter(|| {
                let mut tracker = FieldHeapTracker::new();
                let heap_site = tracker.register_heap_allocation("test_heap");

                for i in 0..stores {
                    tracker.add_heap_store(
                        RefId(i),
                        FieldPath::named(Text::from(format!("field_{}", i % 10))),
                        heap_site,
                        true,
                    );
                }

                black_box(tracker);
            });
        });
    }

    group.finish();
}

// ==================================================================================
// Benchmark 4: Field Heap Analysis (Complexity Test)
// ==================================================================================

fn bench_field_heap_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_heap_analysis");

    // Test O(fields × stores) complexity
    for (fields, stores) in [(2, 10), (5, 20), (10, 50), (20, 100)].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}fields_{}stores", fields, stores)),
            &(*fields, *stores),
            |b, &(fields, stores)| {
                b.iter(|| {
                    let mut tracker = FieldHeapTracker::new();

                    // Register fields
                    let mut field_paths = Set::new();
                    for i in 0..fields {
                        field_paths.insert(FieldPath::named(Text::from(format!("field_{}", i))));
                    }
                    tracker.register_fields(RefId(1), field_paths);

                    // Add heap stores
                    let heap_site = tracker.register_heap_allocation("test");
                    for i in 0..stores {
                        tracker.add_heap_store(
                            RefId(1),
                            FieldPath::named(Text::from(format!("field_{}", i % fields))),
                            heap_site,
                            true,
                        );
                    }

                    // Analyze
                    let result = tracker.track_field_heap_allocations(RefId(1));
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 5: Field Escape Query
// ==================================================================================

fn bench_field_escape_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_escape_query");

    // Prepare tracker with various store counts
    for stores in [10, 100, 1000].iter() {
        let mut tracker = FieldHeapTracker::new();
        let heap_site = tracker.register_heap_allocation("heap");

        for i in 0..*stores {
            tracker.add_heap_store(
                RefId(1),
                FieldPath::named(Text::from(format!("field_{}", i))),
                heap_site,
                true,
            );
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(stores),
            &tracker,
            |b, tracker| {
                b.iter(|| {
                    let escapes = tracker.field_escapes_to_heap(
                        black_box(RefId(1)),
                        black_box(&FieldPath::named(Text::from("field_50"))),
                    );
                    black_box(escapes);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 6: Statistics Computation
// ==================================================================================

fn bench_statistics_computation(c: &mut Criterion) {
    let mut group = c.benchmark_group("statistics_computation");

    for complexity in [10, 100, 500].iter() {
        let mut tracker = FieldHeapTracker::new();

        // Add heap sites and stores
        for i in 0..*complexity {
            tracker.register_heap_allocation(format!("heap_{}", i));
            tracker.add_heap_store(RefId(i), FieldPath::new(), HeapSiteId(i as u64), true);
            tracker.register_fields(RefId(i), Set::new());
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(complexity),
            &tracker,
            |b, tracker| {
                b.iter(|| {
                    let stats = tracker.statistics();
                    black_box(stats);
                });
            },
        );
    }

    group.finish();
}

// ==================================================================================
// Benchmark 7: End-to-End Realistic Workload
// ==================================================================================

fn bench_realistic_workload(c: &mut Criterion) {
    c.bench_function("realistic_workload", |b| {
        b.iter(|| {
            let mut tracker = FieldHeapTracker::new();

            // Simulate realistic struct with 5 fields
            let mut fields = Set::new();
            for field in ["cache", "count", "data", "metadata", "flags"] {
                fields.insert(FieldPath::named(field.into()));
            }
            tracker.register_fields(RefId(1), fields);

            // Simulate 3 heap allocation sites
            let box_heap = tracker.register_heap_allocation("Box::new");
            let vec_heap = tracker.register_heap_allocation("Vec::push");
            let arc_heap = tracker.register_heap_allocation("Arc::new");

            // Add realistic heap stores (cache escapes to all, data to some)
            tracker.add_heap_store(RefId(1), FieldPath::named("cache".into()), box_heap, true);
            tracker.add_heap_store(RefId(1), FieldPath::named("cache".into()), vec_heap, true);
            tracker.add_heap_store(RefId(1), FieldPath::named("cache".into()), arc_heap, true);
            tracker.add_heap_store(RefId(1), FieldPath::named("data".into()), vec_heap, true);

            // Perform analysis
            let result = tracker.track_field_heap_allocations(RefId(1));

            // Query results
            for field in ["cache", "count", "data", "metadata", "flags"] {
                let path = FieldPath::named(field.into());
                let escapes = result.field_escapes_to_heap(&path);
                black_box(escapes);
            }

            // Get statistics
            let stats = result.promotion_rate();
            black_box(stats);
        });
    });
}

criterion_group!(
    benches,
    bench_tracker_creation,
    bench_heap_allocation_registration,
    bench_heap_store_addition,
    bench_field_heap_analysis,
    bench_field_escape_query,
    bench_statistics_computation,
    bench_realistic_workload,
);

criterion_main!(benches);
