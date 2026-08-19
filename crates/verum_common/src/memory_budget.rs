//! A memory ceiling the process enforces on ITSELF.
//!
//! # Why this exists
//!
//! On macOS `ulimit -v` does not work. It reports
//! `setrlimit failed: invalid argument`, leaves the limit `unlimited`,
//! and a process then allocates freely past the number that was asked
//! for — verified: 700 MB allocated under a "512 MB" limit. So the
//! usual outer guard is not available on the platform this compiler is
//! developed on, and a runaway allocation takes the whole machine with
//! it: 128 GB of RAM exhausted, swap full, the desktop unusable until
//! something is killed by hand. That has happened repeatedly.
//!
//! A process cannot rely on its environment to stop it, so it stops
//! itself. This allocator counts live bytes and aborts with a legible
//! message when the total crosses a ceiling — a crash with a reason
//! instead of a machine that has to be power-cycled.
//!
//! # What the ceiling is not
//!
//! It is not a fix for whatever allocates too much; it is the seatbelt.
//! The number it prints — the peak at the moment of death — is the
//! first fact any such investigation needs, and it is exactly what an
//! OOM-killed process fails to leave behind.
//!
//! # Use
//!
//! ```ignore
//! use verum_common::memory_budget::BudgetedAllocator;
//!
//! #[global_allocator]
//! static ALLOC: BudgetedAllocator = BudgetedAllocator::new();
//! ```
//!
//! The ceiling comes from `VERUM_MEMORY_CEILING_GB`, defaulting to
//! [`DEFAULT_CEILING_GB`]. `VERUM_MEMORY_CEILING_GB=0` disables the
//! check for a deliberate large run.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Ceiling used when `VERUM_MEMORY_CEILING_GB` is unset.
///
/// Chosen against the measured peak of the heaviest thing this
/// toolchain does — the stdlib bake, which peaks around 8 GB — with
/// room for growth, and far below the point where a 128 GB machine
/// starts swapping to death.
pub const DEFAULT_CEILING_GB: usize = 24;

const BYTES_PER_GB: usize = 1024 * 1024 * 1024;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);
static ABORTING: AtomicBool = AtomicBool::new(false);

/// Live bytes currently allocated through the budgeted allocator.
pub fn live_bytes() -> usize {
    LIVE_BYTES.load(Ordering::Relaxed)
}

/// The high-water mark since process start.
pub fn peak_bytes() -> usize {
    PEAK_BYTES.load(Ordering::Relaxed)
}

fn ceiling_bytes() -> usize {
    static CEILING: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CEILING.get_or_init(|| {
        let gb = std::env::var("VERUM_MEMORY_CEILING_GB")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT_CEILING_GB);
        gb.saturating_mul(BYTES_PER_GB)
    })
}

/// A `System` allocator that counts, and refuses to help the process
/// exhaust the machine.
pub struct BudgetedAllocator;

impl Default for BudgetedAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl BudgetedAllocator {
    pub const fn new() -> Self {
        BudgetedAllocator
    }

    #[inline]
    fn account(&self, added: usize) {
        let live = LIVE_BYTES.fetch_add(added, Ordering::Relaxed) + added;
        // `fetch_max` keeps the peak truthful under threads without a
        // lock; a slightly stale peak is acceptable, a torn one is not.
        PEAK_BYTES.fetch_max(live, Ordering::Relaxed);

        let ceiling = ceiling_bytes();
        if ceiling == 0 || live <= ceiling {
            return;
        }
        // One thread reports; the rest would only interleave their
        // messages into the same stderr while the machine is already in
        // trouble.
        if ABORTING.swap(true, Ordering::SeqCst) {
            return;
        }
        let gb = |b: usize| b as f64 / BYTES_PER_GB as f64;
        eprintln!(
            "\nverum: MEMORY CEILING REACHED — aborting instead of exhausting the machine.\n\
             \x20 live      {:.1} GB\n\
             \x20 ceiling   {:.1} GB  (VERUM_MEMORY_CEILING_GB)\n\
             \n\
             This is a seatbelt, not a diagnosis: something allocated far more than\n\
             this toolchain's heaviest measured step (the stdlib bake, ~8 GB peak).\n\
             Raise the ceiling for a deliberate large run, or set it to 0 to disable:\n\
             \x20 VERUM_MEMORY_CEILING_GB=48 <command>\n",
            gb(live),
            gb(ceiling),
        );
        std::process::abort();
    }
}

// SAFETY: every method forwards to `System`, which satisfies the
// `GlobalAlloc` contract; the added work is accounting on atomics and
// never touches the returned pointers.
unsafe impl GlobalAlloc for BudgetedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            self.account(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            self.account(layout.size());
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                self.account(new_size - layout.size());
            } else {
                LIVE_BYTES.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[cfg(test)]
mod tests {
    //! The accounting is what the ceiling is made of, so it is what
    //! gets pinned. The abort itself is deliberately not exercised:
    //! a test that aborts the process takes the test runner with it.
    //!
    //! The allocator has to be INSTALLED for any of this to be
    //! measurable — the counters only move for allocations that go
    //! through it, and a library crate installs none. The first version
    //! of these tests asserted against an uninstalled allocator and
    //! failed, which is the honest outcome: without this line the
    //! module under test is not the module doing the work.

    use super::*;

    #[global_allocator]
    static TEST_ALLOC: BudgetedAllocator = BudgetedAllocator::new();

    /// An allocation is counted, and giving it back un-counts it.
    #[test]
    fn live_bytes_follow_allocation_and_release() {
        let before = live_bytes();
        let block = vec![0u8; 4 * 1024 * 1024];
        let during = live_bytes();
        assert!(
            during >= before + block.len(),
            "4 MiB allocated but live went {} -> {}",
            before,
            during
        );
        drop(block);
        assert!(
            live_bytes() < during,
            "release did not reduce the live count"
        );
    }

    /// The peak never goes down — that is what makes it the number
    /// worth printing when something dies.
    #[test]
    fn peak_is_monotonic() {
        let first = peak_bytes();
        drop(vec![0u8; 8 * 1024 * 1024]);
        let after = peak_bytes();
        assert!(after >= first, "peak fell from {} to {}", first, after);
    }

    /// The default is a real number of bytes, not a placeholder — a
    /// zero default would silently disable the seatbelt everywhere.
    #[test]
    fn default_ceiling_is_a_usable_size() {
        assert!(DEFAULT_CEILING_GB >= 8, "ceiling below the bake's own peak");
        assert!(
            DEFAULT_CEILING_GB * BYTES_PER_GB > 0,
            "ceiling overflows to zero"
        );
    }
}
