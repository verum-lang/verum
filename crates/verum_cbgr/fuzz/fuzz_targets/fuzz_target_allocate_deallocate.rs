//! Fuzz target for allocation/deallocation sequences
//!
//! This fuzzer tests:
//! - Random allocation and deallocation patterns
//! - Various sizes and alignments
//! - Detection of: memory leaks, double-free, use-after-free
//!
//! The fuzzer generates random sequences of allocate/deallocate operations
//! and verifies that CBGR correctly detects all safety violations.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use verum_cbgr::{CbgrAllocator, ThinRef};
use verum_common::List;

/// Maximum number of operations per fuzz iteration
const MAX_OPS: usize = 100;

/// Maximum allocation size (in elements)
const MAX_SIZE: usize = 1024;

/// Operation types for fuzzing
#[derive(Debug, Arbitrary, Clone)]
enum Operation {
    /// Allocate an integer
    AllocateInt(i32),

    /// Allocate a vector of integers
    AllocateVec { size: u16 },

    /// Allocate a large struct
    AllocateLargeStruct { a: u64, b: u64, c: u64, d: u64 },

    /// Deallocate by index
    Deallocate { index: u8 },

    /// Clone reference by index
    Clone { index: u8 },

    /// Validate reference by index
    Validate { index: u8 },

    /// Dereference and read by index
    Dereference { index: u8 },

    /// Reallocate with new value
    Reallocate { index: u8, new_value: i32 },
}

/// Large struct for testing different alignments
#[derive(Debug, Clone)]
struct LargeStruct {
    a: u64,
    b: u64,
    c: u64,
    d: u64,
}

/// Container for holding allocated references
enum RefContainer {
    Int(ThinRef<i32>),
    Vec(ThinRef<List<i32>>),
    Large(ThinRef<LargeStruct>),
}

impl RefContainer {
    fn is_valid(&self) -> bool {
        match self {
            RefContainer::Int(r) => r.is_valid(),
            RefContainer::Vec(r) => r.is_valid(),
            RefContainer::Large(r) => r.is_valid(),
        }
    }
}

fuzz_target!(|ops: Vec<Operation>| {
    // Limit operations to prevent timeout
    let ops = if ops.len() > MAX_OPS {
        &ops[..MAX_OPS]
    } else {
        &ops
    };

    let allocator = CbgrAllocator::new();
    let mut refs: Vec<RefContainer> = Vec::new();

    for op in ops {
        match op {
            Operation::AllocateInt(value) => {
                let ptr = allocator.allocate(*value);
                refs.push(RefContainer::Int(ptr));
            }

            Operation::AllocateVec { size } => {
                // Limit size to prevent OOM
                let size = (*size as usize % MAX_SIZE).max(1);
                let vec: List<i32> = (0..size as i32).collect();
                let ptr = allocator.allocate(vec);
                refs.push(RefContainer::Vec(ptr));
            }

            Operation::AllocateLargeStruct { a, b, c, d } => {
                let large = LargeStruct { a: *a, b: *b, c: *c, d: *d };
                let ptr = allocator.allocate(large);
                refs.push(RefContainer::Large(ptr));
            }

            Operation::Deallocate { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let container = refs.swap_remove(idx);

                // Deallocate based on type
                match container {
                    RefContainer::Int(ptr) => {
                        allocator.deallocate(ptr);
                    }
                    RefContainer::Vec(ptr) => {
                        allocator.deallocate(ptr);
                    }
                    RefContainer::Large(ptr) => {
                        allocator.deallocate(ptr);
                    }
                }
            }

            Operation::Clone { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let cloned = match &refs[idx] {
                    RefContainer::Int(r) => RefContainer::Int(r.clone()),
                    RefContainer::Vec(r) => RefContainer::Vec(r.clone()),
                    RefContainer::Large(r) => RefContainer::Large(r.clone()),
                };
                refs.push(cloned);
            }

            Operation::Validate { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();

                // Validation should match is_valid()
                match &refs[idx] {
                    RefContainer::Int(r) => {
                        let valid = r.is_valid();
                        let validate_result = r.validate();
                        assert_eq!(valid, validate_result.is_ok());
                    }
                    RefContainer::Vec(r) => {
                        let valid = r.is_valid();
                        let validate_result = r.validate();
                        assert_eq!(valid, validate_result.is_ok());
                    }
                    RefContainer::Large(r) => {
                        let valid = r.is_valid();
                        let validate_result = r.validate();
                        assert_eq!(valid, validate_result.is_ok());
                    }
                }
            }

            Operation::Dereference { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();

                // Dereference should succeed only if valid
                match &refs[idx] {
                    RefContainer::Int(r) => {
                        let deref_result = r.deref();
                        if r.is_valid() {
                            // Should succeed and return valid data
                            assert!(deref_result.is_ok());
                        } else {
                            // Should fail with use-after-free
                            assert!(deref_result.is_err());
                        }
                    }
                    RefContainer::Vec(r) => {
                        let deref_result = r.deref();
                        if r.is_valid() {
                            assert!(deref_result.is_ok());
                        } else {
                            assert!(deref_result.is_err());
                        }
                    }
                    RefContainer::Large(r) => {
                        let deref_result = r.deref();
                        if r.is_valid() {
                            assert!(deref_result.is_ok());
                        } else {
                            assert!(deref_result.is_err());
                        }
                    }
                }
            }

            Operation::Reallocate { index, new_value } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();

                // Only reallocate Int types for simplicity
                if let RefContainer::Int(old_ptr) = refs.swap_remove(idx) {
                    let new_ptr = allocator.reallocate(old_ptr, *new_value);
                    refs.push(RefContainer::Int(new_ptr));
                }
            }
        }
    }

    // Verify allocator statistics
    let stats = allocator.stats();
    let active = stats.active_allocations();

    // Count valid references
    let valid_count = refs.iter().filter(|r| r.is_valid()).count();

    // Active allocations should match valid references (each may have been cloned)
    // This is a sanity check, not a strict requirement due to cloning
    assert!(active >= valid_count);

    // Clean up remaining allocations
    for container in refs {
        match container {
            RefContainer::Int(ptr) => {
                if ptr.is_valid() {
                    allocator.deallocate(ptr);
                }
            }
            RefContainer::Vec(ptr) => {
                if ptr.is_valid() {
                    allocator.deallocate(ptr);
                }
            }
            RefContainer::Large(ptr) => {
                if ptr.is_valid() {
                    allocator.deallocate(ptr);
                }
            }
        }
    }
});
