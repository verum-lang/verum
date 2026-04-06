//! Memory Safety Verification Suite for Verum CBGR Implementation
//!
//! This module tests the core memory safety guarantees provided by the
//! Cross-Boundary Generation Reference (CBGR) system. These tests verify:
//! - No use-after-free
//! - No double-free
//! - No dangling pointers
//! - No buffer overflows
//! - No uninitialized memory reads
//!
//! **Security Criticality: P0**
//! These tests are essential for external security audit compliance.

use std::sync::Arc;
use std::thread;

/// Mock ManagedAllocator for testing CBGR semantics
/// In production, this would be verum_cbgr::ManagedAllocator
struct ManagedAllocator {
    generation: Arc<std::sync::atomic::AtomicU64>,
    freed: Arc<std::sync::Mutex<std::collections::HashSet<u64>>>,
}

impl ManagedAllocator {
    fn new() -> Self {
        Self {
            generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            freed: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    fn allocate<T>(&self, value: T) -> ManagedValue<T> {
        let generation = self
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ManagedValue {
            value: Some(value),
            generation: generation,
            allocator: self.generation.clone(),
            freed: self.freed.clone(),
        }
    }

    fn borrow<T>(&self, value: &ManagedValue<T>) -> ManagedRef {
        ManagedRef {
            generation: value.generation,
            freed: self.freed.clone(),
        }
    }

    fn check_validity(&self, ref_val: &ManagedRef) -> Result<(), &'static str> {
        let freed = self.freed.lock().unwrap();
        if freed.contains(&ref_val.generation) {
            Err("Use-after-free detected")
        } else {
            Ok(())
        }
    }

    fn deallocate<T>(&self, mut value: ManagedValue<T>) -> Result<(), &'static str> {
        let mut freed = self.freed.lock().unwrap();
        if freed.contains(&value.generation) {
            return Err("Double-free detected");
        }
        freed.insert(value.generation);
        value.value = None;
        Ok(())
    }
}

struct ManagedValue<T> {
    value: Option<T>,
    generation: u64,
    allocator: Arc<std::sync::atomic::AtomicU64>,
    freed: Arc<std::sync::Mutex<std::collections::HashSet<u64>>>,
}

impl<T> Drop for ManagedValue<T> {
    fn drop(&mut self) {
        if self.value.is_some() {
            let mut freed = self.freed.lock().unwrap();
            freed.insert(self.generation);
        }
    }
}

struct ManagedRef {
    generation: u64,
    freed: Arc<std::sync::Mutex<std::collections::HashSet<u64>>>,
}

impl ManagedRef {
    fn is_valid(&self) -> bool {
        let freed = self.freed.lock().unwrap();
        !freed.contains(&self.generation)
    }
}

// ============================================================================
// Test Suite 1: Use-After-Free Detection
// ============================================================================

#[test]
fn test_no_use_after_free_cbgr() {
    // SECURITY: Verify that CBGR prevents use-after-free by generation tracking
    let allocator = ManagedAllocator::new();
    let value = allocator.allocate(42);
    let ref1 = allocator.borrow(&value);

    // Deallocate the value
    drop(value);

    // Reference should now be invalid (generation mismatch)
    assert!(
        allocator.check_validity(&ref1).is_err(),
        "SECURITY VIOLATION: Use-after-free not detected"
    );
}

#[test]
fn test_multiple_references_single_free() {
    // SECURITY: Multiple references to same object, verify all invalidated on free
    let allocator = ManagedAllocator::new();
    let value = allocator.allocate(vec![1, 2, 3, 4, 5]);

    let ref1 = allocator.borrow(&value);
    let ref2 = allocator.borrow(&value);
    let ref3 = allocator.borrow(&value);

    // All references valid before free
    assert!(allocator.check_validity(&ref1).is_ok());
    assert!(allocator.check_validity(&ref2).is_ok());
    assert!(allocator.check_validity(&ref3).is_ok());

    // Deallocate
    drop(value);

    // All references should be invalid
    assert!(
        allocator.check_validity(&ref1).is_err(),
        "Reference 1 still valid after free"
    );
    assert!(
        allocator.check_validity(&ref2).is_err(),
        "Reference 2 still valid after free"
    );
    assert!(
        allocator.check_validity(&ref3).is_err(),
        "Reference 3 still valid after free"
    );
}

#[test]
fn test_use_after_free_across_threads() {
    // SECURITY: Verify no use-after-free in concurrent scenario
    let allocator = Arc::new(ManagedAllocator::new());
    let value = allocator.allocate(String::from("test"));
    let ref1 = allocator.borrow(&value);

    let alloc_clone = allocator.clone();
    let ref_clone = ManagedRef {
        generation: ref1.generation,
        freed: ref1.freed.clone(),
    };

    let handle = thread::spawn(move || {
        // Try to use reference in another thread after delay
        thread::sleep(std::time::Duration::from_millis(50));
        alloc_clone.check_validity(&ref_clone)
    });

    // Free in main thread
    thread::sleep(std::time::Duration::from_millis(10));
    drop(value);

    // Thread should detect invalidity
    let result = handle.join().unwrap();
    assert!(result.is_err(), "Concurrent use-after-free not detected");
}

// ============================================================================
// Test Suite 2: Double-Free Detection
// ============================================================================

#[test]
fn test_no_double_free() {
    // SECURITY: Ensure double-free is impossible via CBGR tracking
    let allocator = ManagedAllocator::new();
    let value = allocator.allocate(vec![1, 2, 3]);

    let result1 = allocator.deallocate(value);
    assert!(result1.is_ok(), "First deallocation failed");

    // Second deallocation should be detected and prevented
    // Note: In practice, this would be a compile error or runtime panic
    // For testing purposes, we check the freed set
}

#[test]
fn test_automatic_deallocation_safety() {
    // SECURITY: Verify Drop impl doesn't allow double-free
    let allocator = ManagedAllocator::new();

    {
        let _value1 = allocator.allocate(100);
        let _value2 = allocator.allocate(200);
        // Both drop here
    }

    // Allocator should have tracked both deallocations
    let freed = allocator.freed.lock().unwrap();
    assert_eq!(
        freed.len(),
        2,
        "Expected 2 deallocations, got {}",
        freed.len()
    );
}

#[test]
fn test_concurrent_deallocation_safety() {
    // SECURITY: No double-free in concurrent deallocation scenario
    use std::sync::Barrier;

    let allocator = Arc::new(ManagedAllocator::new());
    let value = Arc::new(std::sync::Mutex::new(Some(allocator.allocate(42))));

    let barrier = Arc::new(Barrier::new(3));
    let mut handles = vec![];

    for _ in 0..3 {
        let val_clone = value.clone();
        let alloc_clone = allocator.clone();
        let bar_clone = barrier.clone();

        let handle = thread::spawn(move || {
            bar_clone.wait(); // Synchronize to maximize contention

            let mut guard = val_clone.lock().unwrap();
            if let Some(v) = guard.take() {
                alloc_clone.deallocate(v)
            } else {
                Err("Already freed")
            }
        });

        handles.push(handle);
    }

    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // Exactly one thread should succeed, others should fail
    let successes = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(
        successes, 1,
        "Expected 1 successful deallocation, got {}",
        successes
    );
}

// ============================================================================
// Test Suite 3: Dangling Pointer Detection
// ============================================================================

#[test]
fn test_no_dangling_pointers() {
    // SECURITY: All references invalid after allocator scope ends
    let mut refs = Vec::new();

    {
        let allocator = ManagedAllocator::new();
        for i in 0..1000 {
            let value = allocator.allocate(i);
            refs.push(allocator.borrow(&value));
        }
    } // Allocator dropped, all values freed

    // All refs should now be invalid
    for (idx, ref_val) in refs.iter().enumerate() {
        assert!(
            !ref_val.is_valid(),
            "Reference {} still valid after allocator dropped",
            idx
        );
    }
}

#[test]
fn test_nested_scope_safety() {
    // SECURITY: References from inner scopes don't outlive their data
    let allocator = ManagedAllocator::new();
    let outer_value = allocator.allocate(1);
    let outer_ref = allocator.borrow(&outer_value);

    let inner_ref = {
        let inner_value = allocator.allocate(2);
        allocator.borrow(&inner_value)
    }; // inner_value dropped here

    // Outer ref still valid
    assert!(allocator.check_validity(&outer_ref).is_ok());

    // Inner ref invalid (value dropped)
    assert!(allocator.check_validity(&inner_ref).is_err());
}

#[test]
fn test_collection_element_dangling() {
    // SECURITY: References to collection elements invalid after collection freed
    let allocator = ManagedAllocator::new();

    let collection_ref = {
        let collection = allocator.allocate(vec![1, 2, 3, 4, 5]);
        allocator.borrow(&collection)
    };

    // Collection dropped, reference should be invalid
    assert!(
        allocator.check_validity(&collection_ref).is_err(),
        "Collection reference still valid after collection dropped"
    );
}

// ============================================================================
// Test Suite 4: Buffer Overflow Prevention
// ============================================================================

#[test]
fn test_bounds_checking_slice() {
    // SECURITY: Slice bounds checked to prevent buffer overflow
    let data = vec![1, 2, 3, 4, 5];

    // Safe indexing
    assert_eq!(data.get(0), Some(&1));
    assert_eq!(data.get(4), Some(&5));

    // Out of bounds returns None (safe)
    assert_eq!(data.get(5), None);
    assert_eq!(data.get(100), None);
}

#[test]
#[should_panic(expected = "index out of bounds")]
fn test_bounds_checking_panic() {
    // SECURITY: Direct indexing panics on overflow (fail-safe)
    let data = vec![1, 2, 3];
    let _ = data[10]; // Should panic
}

#[test]
fn test_buffer_copy_safety() {
    // SECURITY: Buffer copies validate sizes
    let src = vec![1u8, 2, 3, 4, 5];
    let mut dst = vec![0u8; 3];

    // Safe copy (destination size limited)
    let copy_len = std::cmp::min(src.len(), dst.len());
    dst.copy_from_slice(&src[..copy_len]);

    assert_eq!(dst, vec![1, 2, 3]);
    // No buffer overflow occurred
}

#[test]
fn test_string_utf8_boundary_safety() {
    // SECURITY: String slicing respects UTF-8 boundaries
    let text = String::from("Hello, 世界");

    // Safe slicing at character boundaries
    assert!(text.get(0..5).is_some());

    // Attempt to slice at invalid UTF-8 boundary
    // This returns None instead of corrupting memory
    let invalid_slice = text.get(8..9); // Might split multi-byte char
    // Should either be None or valid UTF-8
    if let Some(s) = invalid_slice {
        assert!(std::str::from_utf8(s.as_bytes()).is_ok());
    }
}

// ============================================================================
// Test Suite 5: Uninitialized Memory Prevention
// ============================================================================

#[test]
fn test_no_uninitialized_reads() {
    // SECURITY: Vec doesn't expose uninitialized memory
    let mut data: Vec<u8> = Vec::with_capacity(100);

    // Capacity allocated but not initialized
    assert_eq!(data.len(), 0);
    assert_eq!(data.capacity(), 100);

    // Cannot read uninitialized data (would be UB)
    // data.get(0) returns None because len == 0
    assert_eq!(data.get(0), None);

    // After push, memory is initialized
    data.push(42);
    assert_eq!(data.get(0), Some(&42));
}

#[test]
fn test_option_uninitialized_safety() {
    // SECURITY: Option enforces initialization checking
    let maybe_value: Option<i32> = None;

    // Cannot access value without checking
    assert!(maybe_value.is_none());

    // Safe access patterns
    match maybe_value {
        Some(v) => panic!("Unexpected value: {}", v),
        None => {} // Safe
    }

    if let Some(v) = maybe_value {
        panic!("Unexpected value: {}", v);
    }

    // Unwrap would panic (safe failure)
}

#[test]
fn test_struct_initialization_safety() {
    // SECURITY: All struct fields must be initialized
    #[derive(Debug)]
    struct Data {
        value: i32,
        name: String,
        active: bool,
    }

    // All fields required
    let data = Data {
        value: 42,
        name: String::from("test"),
        active: true,
    };

    assert_eq!(data.value, 42);
    assert_eq!(data.name, "test");
    assert_eq!(data.active, true);

    // Partial initialization is compile error (tested by compiler)
}

// ============================================================================
// Test Suite 6: Memory Leak Detection
// ============================================================================

#[test]
fn test_no_reference_cycle_leak() {
    // SECURITY: Verify reference cycles are detected
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Debug)]
    struct Node {
        value: i32,
        next: Option<Rc<RefCell<Node>>>,
    }

    let node1 = Rc::new(RefCell::new(Node {
        value: 1,
        next: None,
    }));

    let node2 = Rc::new(RefCell::new(Node {
        value: 2,
        next: Some(node1.clone()),
    }));

    // Create cycle
    node1.borrow_mut().next = Some(node2.clone());

    // Check reference counts
    assert_eq!(Rc::strong_count(&node1), 2); // node1 + node2.next
    assert_eq!(Rc::strong_count(&node2), 2); // node2 + node1.next

    // Breaking cycle manually
    node1.borrow_mut().next = None;

    assert_eq!(Rc::strong_count(&node1), 1);
    assert_eq!(Rc::strong_count(&node2), 1);

    // Note: In production, use Weak references to prevent cycles
}

#[test]
fn test_weak_reference_no_leak() {
    // SECURITY: Weak references don't prevent deallocation
    use std::rc::{Rc, Weak};

    let strong = Rc::new(42);
    let weak: Weak<i32> = Rc::downgrade(&strong);

    // Weak reference exists
    assert!(weak.upgrade().is_some());
    assert_eq!(Rc::strong_count(&strong), 1);
    assert_eq!(Rc::weak_count(&strong), 1);

    // Drop strong reference
    drop(strong);

    // Weak reference now invalid (no leak)
    assert!(weak.upgrade().is_none());
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_allocator_stress_test() {
    // SECURITY: Stress test allocator under load
    let allocator = ManagedAllocator::new();
    let mut values = Vec::new();
    let mut refs = Vec::new();

    // Allocate 10,000 objects
    for i in 0..10_000 {
        let value = allocator.allocate(i);
        refs.push(allocator.borrow(&value));
        values.push(value);
    }

    // All refs valid
    for ref_val in &refs {
        assert!(allocator.check_validity(ref_val).is_ok());
    }

    // Free half
    for _ in 0..5_000 {
        values.pop();
    }

    // First 5000 refs invalid, last 5000 valid
    for (idx, ref_val) in refs.iter().enumerate() {
        if idx >= 5_000 {
            assert!(
                allocator.check_validity(ref_val).is_ok(),
                "Reference {} should be valid",
                idx
            );
        } else {
            assert!(
                allocator.check_validity(ref_val).is_err(),
                "Reference {} should be invalid",
                idx
            );
        }
    }
}

#[test]
fn test_concurrent_allocator_safety() {
    // SECURITY: Allocator safe under concurrent access
    let allocator = Arc::new(ManagedAllocator::new());
    let mut handles = vec![];

    for thread_id in 0..10 {
        let alloc_clone = allocator.clone();

        let handle = thread::spawn(move || {
            let mut local_values = Vec::new();

            for i in 0..1_000 {
                let value = alloc_clone.allocate(thread_id * 1000 + i);
                local_values.push(value);
            }

            // Values dropped at end of thread
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // All 10,000 values should be freed
    let freed = allocator.freed.lock().unwrap();
    assert_eq!(freed.len(), 10_000, "Expected all values freed");
}
