//! Real allocation/deallocation below an already reached high-water mark.
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::alloc::{GlobalAlloc, Layout, handle_alloc_error};
use std::hint::black_box;
use std::sync::{
    Barrier,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use verum_common::memory_budget::{BudgetedAllocator, live_bytes, peak_bytes};

// Account the measured allocations without counting Criterion's own storage.
static ALLOC: BudgetedAllocator = BudgetedAllocator::new();

fn allocation_cycle(layout: Layout) {
    // SAFETY: the nonzero layout is valid, and each cycle owns the returned
    // pointer exclusively until it is released with its original layout.
    unsafe {
        let pointer = ALLOC.alloc(layout);
        if pointer.is_null() {
            handle_alloc_error(layout);
        }
        black_box(pointer);
        ALLOC.dealloc(pointer, layout);
    }
}

fn bench_budget(c: &mut Criterion) {
    let retained_peak = 1024 * 1024;
    allocation_cycle(Layout::from_size_align(retained_peak, 64).unwrap());
    assert_eq!(live_bytes(), 0);
    assert_eq!(peak_bytes(), retained_peak);
    let layout = Layout::from_size_align(256, 64).unwrap();
    let mut group = c.benchmark_group("memory_budget");
    group.sample_size(20);
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_secs(2));
    const CYCLES: usize = 8192;
    // Isolate accounting from worker wakeup/scheduling and keep both ordinary
    // and over-aligned system allocation paths visible in the comparison.
    group.throughput(Throughput::Elements(CYCLES as u64));
    for (name, alignment) in [
        ("natural", std::mem::align_of::<usize>()),
        ("overaligned", 64),
    ] {
        let layout = Layout::from_size_align(256, alignment).unwrap();
        group.bench_function(BenchmarkId::new("sequential_below_peak", name), |b| {
            b.iter(|| {
                for _ in 0..CYCLES {
                    allocation_cycle(layout);
                }
            });
        });
        assert_eq!(live_bytes(), 0);
        assert_eq!(peak_bytes(), retained_peak);
    }
    for workers in [1, 2, 8] {
        group.throughput(Throughput::Elements((workers * CYCLES) as u64));
        let ready = Barrier::new(workers + 1);
        let finished = Barrier::new(workers + 1);
        let stop = AtomicBool::new(false);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let (ready, finished, stop) = (&ready, &finished, &stop);
                scope.spawn(move || {
                    loop {
                        ready.wait();
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        for _ in 0..CYCLES {
                            allocation_cycle(layout);
                        }
                        finished.wait();
                    }
                });
            }
            group.bench_function(BenchmarkId::new("below_peak", workers), |b| {
                b.iter(|| {
                    ready.wait();
                    finished.wait();
                });
            });
            stop.store(true, Ordering::Release);
            ready.wait();
        });
        assert_eq!(live_bytes(), 0);
        assert_eq!(peak_bytes(), retained_peak);
    }
    group.finish();
}

criterion_group!(benches, bench_budget);
criterion_main!(benches);
