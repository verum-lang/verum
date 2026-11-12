//! Exact accounting through a direct allocator, independent of harness allocations.
use std::alloc::{GlobalAlloc, Layout, handle_alloc_error};
use std::ptr::NonNull;
use std::sync::Barrier;
use verum_common::List;
use verum_common::memory_budget::{
    BYTES_PER_GB, BudgetedAllocator, DEFAULT_CEILING_GB, live_bytes, peak_bytes,
};

// Deliberately not the process's global allocator: harness/Barrier/List memory
// must not be subtracted from a different instant or hidden behind a tolerance.
static ALLOC: BudgetedAllocator = BudgetedAllocator::new();

struct Block {
    pointer: NonNull<u8>,
    layout: Layout,
}

impl Block {
    fn new(bytes: usize, zeroed: bool) -> Self {
        let layout = Layout::from_size_align(bytes, 64).unwrap();
        // SAFETY: the nonzero layout is valid; this Block exclusively owns the
        // returned allocation and releases it through the same allocator.
        let raw = unsafe {
            if zeroed {
                ALLOC.alloc_zeroed(layout)
            } else {
                ALLOC.alloc(layout)
            }
        };
        let pointer = NonNull::new(raw).unwrap_or_else(|| handle_alloc_error(layout));
        Self { pointer, layout }
    }

    fn resize(&mut self, bytes: usize) {
        let layout = Layout::from_size_align(bytes, self.layout.align()).unwrap();
        // SAFETY: the pointer is live and exclusively owned, its original
        // layout is retained, and the new size is nonzero with the same alignment.
        let raw = unsafe { ALLOC.realloc(self.pointer.as_ptr(), self.layout, bytes) };
        self.pointer = NonNull::new(raw).unwrap_or_else(|| handle_alloc_error(layout));
        self.layout = layout;
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        // SAFETY: this is the unique live pointer returned by ALLOC with exactly
        // this layout, including the most recent successful reallocation.
        unsafe { ALLOC.dealloc(self.pointer.as_ptr(), self.layout) };
    }
}

#[test]
fn concurrent_accounting_has_exact_live_bytes_and_monotone_high_water() {
    assert_eq!(live_bytes(), 0);
    assert_eq!(peak_bytes(), 0);
    const WORKERS: usize = 8;
    let weight: usize = (1..=WORKERS).sum();
    let mut expected_peak = 0;
    // Lower waves exercise allocations below a retained peak; higher waves
    // require concurrent maxima to advance it. No sampling or timing assumptions.
    for scale in [1, 4, 2, 8, 1] {
        let barrier = Barrier::new(WORKERS + 1);
        let mut observations = List::new();
        std::thread::scope(|scope| {
            for index in 1..=WORKERS {
                let barrier = &barrier;
                scope.spawn(move || {
                    let mut zeroed = Block::new(index * scale * 256, true);
                    let mut plain = Block::new(index * scale * 64, false);
                    barrier.wait();
                    barrier.wait();
                    zeroed.resize(index * scale * 768);
                    barrier.wait();
                    barrier.wait();
                    zeroed.resize(index * scale * 128);
                    plain.resize(index * scale * 32);
                    barrier.wait();
                    barrier.wait();
                    drop(zeroed);
                    drop(plain);
                    barrier.wait();
                    barrier.wait();
                });
            }
            for _ in 0..4 {
                barrier.wait();
                observations.push((live_bytes(), peak_bytes()));
                barrier.wait();
            }
        });
        // Assert after every worker has exited: an assertion failure must not
        // leave other threads trapped at a barrier while the scope joins them.
        for (actual, expected_live) in observations.into_iter().zip([
            weight * scale * 320,
            weight * scale * 832,
            weight * scale * 160,
            0,
        ]) {
            expected_peak = expected_peak.max(expected_live);
            assert_eq!(actual, (expected_live, expected_peak), "scale {scale}");
        }
    }
}

#[test]
fn default_ceiling_is_a_usable_size() {
    assert!(DEFAULT_CEILING_GB >= 8, "ceiling below the bake's own peak");
    assert!(
        DEFAULT_CEILING_GB * BYTES_PER_GB > 0,
        "ceiling overflows to zero"
    );
}
