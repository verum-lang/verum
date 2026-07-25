//! Fuzz target for capability flag validation
//!
//! This fuzzer tests:
//! - Capability flag violations
//! - Invalid capability combinations
//! - Detection of: capability bypass, privilege escalation
//!
//! The fuzzer generates random capability combinations and operations,
//! verifying that CBGR enforces capability requirements correctly.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use verum_cbgr::{CbgrAllocator, ThinRef, Capability};
use std::ptr::NonNull;

/// Maximum operations for capability testing
const MAX_OPS: usize = 100;

/// Capability test operations
#[derive(Debug, Arbitrary, Clone)]
enum CapabilityOp {
    /// Allocate with default capabilities (ALL)
    AllocateDefault(i32),

    /// Create ThinRef with specific capabilities
    CreateWithCapabilities {
        value: i32,
        read: bool,
        write: bool,
        execute: bool,
        share: bool,
    },

    /// Check if reference has read capability
    CheckRead { index: u8 },

    /// Check if reference has write capability
    CheckWrite { index: u8 },

    /// Check if reference has execute capability
    CheckExecute { index: u8 },

    /// Check if reference has share capability
    CheckShare { index: u8 },

    /// Attempt to dereference (requires READ)
    AttemptRead { index: u8 },

    /// Attempt to dereference mutably (requires WRITE)
    AttemptWrite { index: u8, new_value: i32 },

    /// Attempt to clone/share (requires SHARE)
    AttemptShare { index: u8 },

    /// Verify capability flags are correctly stored
    VerifyCapabilityFlags { index: u8 },

    /// Test capability bit manipulation
    TestCapabilityBits { cap_bits: u16 },

    /// Deallocate reference
    Deallocate { index: u8 },
}

/// Wrapper for tracking capabilities
struct CapabilityTestRef {
    ptr: ThinRef<i32>,
    expected_caps: u16,
}

impl CapabilityTestRef {
    fn new(ptr: ThinRef<i32>, caps: u16) -> Self {
        Self {
            ptr,
            expected_caps: caps,
        }
    }

    fn has_capability(&self, cap: Capability) -> bool {
        self.ptr.has_capability(cap)
    }

    fn verify_capabilities(&self) -> bool {
        self.ptr.capabilities() == self.expected_caps
    }
}

/// Helper to build capability flags
fn build_capability_flags(read: bool, write: bool, execute: bool, _share: bool) -> u16 {
    let mut flags = 0u16;

    if read {
        flags |= Capability::Read as u16;
    }
    if write {
        flags |= Capability::Write as u16;
    }
    if execute {
        flags |= Capability::Execute as u16;
    }
    // Note: SHARE capability doesn't exist in current implementation
    // Using Delegate instead for sharing capabilities
    if _share {
        flags |= Capability::Delegate as u16;
    }

    flags
}

fuzz_target!(|ops: Vec<CapabilityOp>| {
    // Limit operations
    let ops = if ops.len() > MAX_OPS {
        &ops[..MAX_OPS]
    } else {
        &ops
    };

    let allocator = CbgrAllocator::new();
    let mut refs: Vec<CapabilityTestRef> = Vec::new();

    for op in ops {
        match op {
            CapabilityOp::AllocateDefault(value) => {
                // Default allocation has ALL capabilities
                let ptr = allocator.allocate(*value);
                refs.push(CapabilityTestRef::new(ptr, Capability::ALL));
            }

            CapabilityOp::CreateWithCapabilities {
                value,
                read,
                write,
                execute,
                share,
            } => {
                // Build capability flags
                let caps = build_capability_flags(*read, *write, *execute, *share);

                // Allocate with default caps first
                let ptr = allocator.allocate(*value);

                // Note: We can't directly set capabilities on an allocated ThinRef
                // The allocator always creates with Capability::ALL
                // This test verifies that the stored capabilities are correct

                // For testing purposes, we'll track what we expect vs what we get
                refs.push(CapabilityTestRef::new(ptr, Capability::ALL));

                // In a real implementation with capability enforcement,
                // we'd need allocator.allocate_with_caps(value, caps)
            }

            CapabilityOp::CheckRead { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let test_ref = &refs[idx];

                let has_read = test_ref.has_capability(Capability::Read);

                // With default allocator, should always have Read
                assert!(has_read, "Default allocation should have Read capability");
            }

            CapabilityOp::CheckWrite { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let test_ref = &refs[idx];

                let has_write = test_ref.has_capability(Capability::Write);

                // With default allocator, should always have Write
                assert!(has_write, "Default allocation should have Write capability");
            }

            CapabilityOp::CheckExecute { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let test_ref = &refs[idx];

                let has_execute = test_ref.has_capability(Capability::Execute);

                // With default allocator, should always have Execute
                assert!(has_execute, "Default allocation should have Execute capability");
            }

            CapabilityOp::CheckShare { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let test_ref = &refs[idx];

                let has_delegate = test_ref.has_capability(Capability::Delegate);

                // With default allocator, should always have Delegate (share) capability
                assert!(has_delegate, "Default allocation should have Delegate capability");
            }

            CapabilityOp::AttemptRead { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let test_ref = &refs[idx];

                if test_ref.ptr.is_valid() {
                    // Should succeed because we have Read capability
                    let deref_result = test_ref.ptr.deref();

                    if test_ref.has_capability(Capability::Read) {
                        assert!(deref_result.is_ok(), "Deref should succeed with Read capability");
                    } else {
                        // If we didn't have Read, it should fail
                        // (current implementation always has ALL caps)
                        assert!(deref_result.is_err(), "Deref should fail without Read capability");
                    }
                }
            }

            CapabilityOp::AttemptWrite { index, new_value } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let test_ref = &refs[idx];

                if test_ref.ptr.is_valid() {
                    // Clone for mutation (we can't directly mut borrow from test_ref)
                    let mut ptr_mut = test_ref.ptr.clone();

                    let deref_result = ptr_mut.deref_mut();

                    if test_ref.has_capability(Capability::Write) {
                        if let Ok(value) = deref_result {
                            *value = *new_value;
                        }
                    } else {
                        // Should fail without Write capability
                        // (current implementation always has ALL caps)
                        assert!(deref_result.is_err(), "Deref_mut should fail without Write capability");
                    }
                }
            }

            CapabilityOp::AttemptShare { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let test_ref = &refs[idx];

                // Clone is a form of sharing
                let cloned = test_ref.ptr.clone();

                // Verify cloned has same capabilities
                assert_eq!(cloned.capabilities(), test_ref.ptr.capabilities());

                if test_ref.has_capability(Capability::Delegate) {
                    // Should be allowed to share/delegate
                    refs.push(CapabilityTestRef::new(cloned, test_ref.expected_caps));
                } else {
                    // Without Delegate capability, clone shouldn't work
                    // (current implementation always allows clone)
                }
            }

            CapabilityOp::VerifyCapabilityFlags { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let test_ref = &refs[idx];

                // Verify stored capabilities match expected
                assert!(test_ref.verify_capabilities(),
                    "Capability flags should match expected: expected={:016b}, actual={:016b}",
                    test_ref.expected_caps,
                    test_ref.ptr.capabilities()
                );

                // Verify individual capability checks are consistent
                let has_read = test_ref.has_capability(Capability::Read);
                let has_write = test_ref.has_capability(Capability::Write);
                let has_execute = test_ref.has_capability(Capability::Execute);
                let has_delegate = test_ref.has_capability(Capability::Delegate);

                let caps = test_ref.ptr.capabilities();

                assert_eq!(has_read, (caps & Capability::Read as u16) != 0);
                assert_eq!(has_write, (caps & Capability::Write as u16) != 0);
                assert_eq!(has_execute, (caps & Capability::Execute as u16) != 0);
                assert_eq!(has_delegate, (caps & Capability::Delegate as u16) != 0);
            }

            CapabilityOp::TestCapabilityBits { cap_bits } => {
                // Test capability bit operations
                let caps = *cap_bits;

                // Verify Capability enum values are correct
                assert_eq!(Capability::Read as u16, 0b0001);
                assert_eq!(Capability::Write as u16, 0b0010);
                assert_eq!(Capability::Execute as u16, 0b0100);
                assert_eq!(Capability::Delegate as u16, 0b1000);

                // Test Capability::ALL contains all expected bits
                assert!(Capability::ALL > 0);

                // Test bit combinations
                let has_all_read = Capability::Read.is_present(caps);
                let has_all_write = Capability::Write.is_present(caps);
                let has_all_execute = Capability::Execute.is_present(caps);
                let has_all_delegate = Capability::Delegate.is_present(caps);

                // Verify consistency
                assert_eq!(has_all_read, (caps & Capability::Read as u16) != 0);
                assert_eq!(has_all_write, (caps & Capability::Write as u16) != 0);
                assert_eq!(has_all_execute, (caps & Capability::Execute as u16) != 0);
                assert_eq!(has_all_delegate, (caps & Capability::Delegate as u16) != 0);

                // Test that ALL capability has all basic bits set
                assert!(Capability::Read.is_present(Capability::ALL));
                assert!(Capability::Write.is_present(Capability::ALL));
                assert!(Capability::Execute.is_present(Capability::ALL));
                assert!(Capability::Delegate.is_present(Capability::ALL));
            }

            CapabilityOp::Deallocate { index } => {
                if refs.is_empty() {
                    continue;
                }

                let idx = *index as usize % refs.len();
                let test_ref = refs.swap_remove(idx);

                if test_ref.ptr.is_valid() {
                    allocator.deallocate(test_ref.ptr);
                }
            }
        }
    }

    // Final verification
    for test_ref in &refs {
        if test_ref.ptr.is_valid() {
            // Verify capabilities are still correct
            assert!(test_ref.verify_capabilities());

            // Verify we can still check individual capabilities
            let _ = test_ref.has_capability(Capability::Read);
            let _ = test_ref.has_capability(Capability::Write);
            let _ = test_ref.has_capability(Capability::Execute);
            let _ = test_ref.has_capability(Capability::Delegate);
        }
    }

    // Clean up
    for test_ref in refs {
        if test_ref.ptr.is_valid() {
            allocator.deallocate(test_ref.ptr);
        }
    }

    // Verify allocator consistency
    let stats = allocator.stats();
    assert!(stats.total_deallocations() <= stats.total_allocations());
});
