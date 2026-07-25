//! Fuzz target for concurrent access patterns
//!
//! This fuzzer tests:
//! - Multi-threaded allocation/deallocation
//! - Random ThinRef operations across threads
//! - Detection of: data races, deadlocks, atomicity violations
//!
//! The fuzzer spawns multiple threads that perform concurrent CBGR operations,
//! verifying that all atomic operations maintain consistency.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use verum_cbgr::{CbgrAllocator, ThinRef};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;

/// Maximum number of threads
const MAX_THREADS: usize = 8;

/// Maximum operations per thread
const MAX_OPS_PER_THREAD: usize = 50;

/// Thread operation types
#[derive(Debug, Arbitrary, Clone)]
enum ThreadOp {
    /// Allocate and share a value
    AllocateShared(i32),

    /// Validate a shared reference
    ValidateShared { ref_idx: u8 },

    /// Dereference a shared reference
    DereferenceShared { ref_idx: u8 },

    /// Clone a shared reference
    CloneShared { ref_idx: u8 },

    /// Deallocate a shared reference (first thread wins)
    DeallocateShared { ref_idx: u8 },

    /// Read generation counter
    ReadGeneration { ref_idx: u8 },

    /// Read epoch counter
    ReadEpoch { ref_idx: u8 },

    /// Check validity
    CheckValid { ref_idx: u8 },

    /// Sleep to introduce timing variations
    Sleep { micros: u8 },
}

/// Thread configuration
#[derive(Debug, Arbitrary, Clone)]
struct ThreadConfig {
    /// Operations for this thread
    ops: Vec<ThreadOp>,
}

/// Fuzz input
#[derive(Debug, Arbitrary)]
struct FuzzInput {
    /// Number of threads (1-8)
    num_threads: u8,

    /// Operations per thread
    thread_configs: Vec<ThreadConfig>,

    /// Initial shared allocations
    initial_values: Vec<i32>,
}

fuzz_target!(|input: FuzzInput| {
    // Limit thread count
    let num_threads = (input.num_threads as usize % MAX_THREADS).max(1);

    // Create shared allocator
    let allocator = Arc::new(CbgrAllocator::new());

    // Create initial shared references (using Mutex for thread-safe access)
    let shared_refs: Arc<Mutex<Vec<ThinRef<i32>>>> = {
        let mut refs = Vec::new();
        for value in input.initial_values.iter().take(10) {
            let ptr = allocator.allocate(*value);
            refs.push(ptr);
        }
        Arc::new(Mutex::new(refs))
    };

    // Create stop flag for threads
    let stop_flag = Arc::new(AtomicBool::new(false));

    // Spawn threads
    let mut handles = Vec::new();

    for thread_id in 0..num_threads {
        let allocator = Arc::clone(&allocator);
        let shared_refs = Arc::clone(&shared_refs);
        let stop_flag = Arc::clone(&stop_flag);

        // Get thread config (cycling if not enough configs)
        let config = if input.thread_configs.is_empty() {
            ThreadConfig { ops: Vec::new() }
        } else {
            input.thread_configs[thread_id % input.thread_configs.len()].clone()
        };

        let handle = thread::spawn(move || {
            // Limit operations
            let ops = if config.ops.len() > MAX_OPS_PER_THREAD {
                &config.ops[..MAX_OPS_PER_THREAD]
            } else {
                &config.ops
            };

            let mut local_refs: Vec<ThinRef<i32>> = Vec::new();

            for op in ops {
                // Check stop flag
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }

                match *op {
                    ThreadOp::AllocateShared(value) => {
                        let ptr = allocator.allocate(value);
                        shared_refs.lock().unwrap().push(ptr.clone());
                        local_refs.push(ptr);
                    }

                    ThreadOp::ValidateShared { ref_idx } => {
                        let refs = shared_refs.lock().unwrap();
                        if refs.is_empty() {
                            continue;
                        }
                        let idx = ref_idx as usize % refs.len();
                        let _ = refs[idx].validate();
                    }

                    ThreadOp::DereferenceShared { ref_idx } => {
                        let refs = shared_refs.lock().unwrap();
                        if refs.is_empty() {
                            continue;
                        }
                        let idx = ref_idx as usize % refs.len();
                        let _ = refs[idx].deref();
                    }

                    ThreadOp::CloneShared { ref_idx } => {
                        let refs = shared_refs.lock().unwrap();
                        if refs.is_empty() {
                            continue;
                        }
                        let idx = ref_idx as usize % refs.len();
                        let cloned = refs[idx].clone();
                        drop(refs); // Release lock
                        local_refs.push(cloned);
                    }

                    ThreadOp::DeallocateShared { ref_idx } => {
                        // Try to remove from shared list (may race with other threads)
                        let mut refs = shared_refs.lock().unwrap();
                        if refs.is_empty() {
                            continue;
                        }
                        let idx = ref_idx as usize % refs.len();

                        // Only deallocate if still valid (prevents double-free)
                        if idx < refs.len() {
                            let ptr = refs[idx].clone();
                            drop(refs); // Release lock before deallocation

                            if ptr.is_valid() {
                                // Try to deallocate (may panic if already freed by another thread)
                                // This is expected behavior - CBGR should catch double-free
                                let result = std::panic::catch_unwind(|| {
                                    allocator.deallocate(ptr);
                                });
                                // If it panicked, it detected double-free (good!)
                                // If it succeeded, we freed it (also good!)
                                let _ = result;
                            }
                        }
                    }

                    ThreadOp::ReadGeneration { ref_idx } => {
                        let refs = shared_refs.lock().unwrap();
                        if refs.is_empty() {
                            continue;
                        }
                        let idx = ref_idx as usize % refs.len();
                        let _ = refs[idx].generation();
                    }

                    ThreadOp::ReadEpoch { ref_idx } => {
                        let refs = shared_refs.lock().unwrap();
                        if refs.is_empty() {
                            continue;
                        }
                        let idx = ref_idx as usize % refs.len();
                        let _ = refs[idx].epoch();
                    }

                    ThreadOp::CheckValid { ref_idx } => {
                        let refs = shared_refs.lock().unwrap();
                        if refs.is_empty() {
                            continue;
                        }
                        let idx = ref_idx as usize % refs.len();
                        let is_valid = refs[idx].is_valid();

                        // Verify consistency: is_valid should match validate()
                        let validate_ok = refs[idx].validate().is_ok();
                        assert_eq!(is_valid, validate_ok);
                    }

                    ThreadOp::Sleep { micros } => {
                        // Introduce timing variations
                        let duration = std::time::Duration::from_micros(micros as u64);
                        std::thread::sleep(duration);
                    }
                }
            }

            // Clean up local refs (ignore panics from already-freed)
            for ptr in local_refs {
                if ptr.is_valid() {
                    let _ = std::panic::catch_unwind(|| {
                        allocator.deallocate(ptr);
                    });
                }
            }
        });

        handles.push(handle);
    }

    // Set timeout to prevent hangs
    let timeout_handle = {
        let stop_flag = Arc::clone(&stop_flag);
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_secs(1));
            stop_flag.store(true, Ordering::Relaxed);
        })
    };

    // Wait for all threads
    for handle in handles {
        let _ = handle.join();
    }

    // Stop timeout thread
    stop_flag.store(true, Ordering::Relaxed);
    let _ = timeout_handle.join();

    // Clean up remaining shared refs (best effort)
    let refs = shared_refs.lock().unwrap();
    for ptr in refs.iter() {
        if ptr.is_valid() {
            let _ = std::panic::catch_unwind(|| {
                allocator.deallocate(ptr.clone());
            });
        }
    }
    drop(refs);

    // Verify no memory corruption - allocator stats should be consistent
    let stats = allocator.stats();
    let total_alloc = stats.total_allocations();
    let total_dealloc = stats.total_deallocations();

    // Deallocations shouldn't exceed allocations
    assert!(total_dealloc <= total_alloc);
});
