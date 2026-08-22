//! Accounting pins for the self-imposed memory ceiling.
//!
//! These lived as inline `#[cfg(test)]` tests and raced: the counters
//! are process-global, the crate's ~190 other unit tests run in
//! parallel in the same process, and "release reduces the live count"
//! is false the instant a neighbour allocates between the two samples
//! — the first CI run that actually executed the unit tier caught it
//! (Unit ubuntu-latest, live_bytes_follow_allocation_and_release).
//! An integration-test binary is its own process: the allocator here
//! counts only what THIS file does, and the lifecycle assertions run
//! in one test, in order.
//!
//! The allocator has to be INSTALLED for any of this to be
//! measurable — the counters only move for allocations that go
//! through it. The abort itself is deliberately not exercised: a test
//! that aborts the process takes the test runner with it.

use verum_common::memory_budget::{
    live_bytes, peak_bytes, BudgetedAllocator, BYTES_PER_GB, DEFAULT_CEILING_GB,
};

#[global_allocator]
static TEST_ALLOC: BudgetedAllocator = BudgetedAllocator::new();

/// An allocation is counted, giving it back un-counts it, and the
/// peak never goes down — one sequential lifecycle, because the three
/// facts share the two global counters.
#[test]
fn accounting_follows_the_allocation_lifecycle() {
    let before = live_bytes();
    let block = vec![0u8; 4 * 1024 * 1024];
    let during = live_bytes();
    assert!(
        during >= before + block.len(),
        "4 MiB allocated but live went {} -> {}",
        before,
        during
    );

    let peak_at_height = peak_bytes();
    assert!(
        peak_at_height >= during,
        "peak {} below a live figure {} it must have witnessed",
        peak_at_height,
        during
    );

    drop(block);
    assert!(
        live_bytes() < during,
        "release did not reduce the live count"
    );
    assert!(
        peak_bytes() >= peak_at_height,
        "peak fell from {} to {} — it is monotonic by construction",
        peak_at_height,
        peak_bytes()
    );
}

/// The default is a real number of bytes, not a placeholder — a zero
/// default would silently disable the seatbelt everywhere.
#[test]
fn default_ceiling_is_a_usable_size() {
    assert!(DEFAULT_CEILING_GB >= 8, "ceiling below the bake's own peak");
    assert!(
        DEFAULT_CEILING_GB * BYTES_PER_GB > 0,
        "ceiling overflows to zero"
    );
}
