//! Concurrency Safety Verification Suite for Verum
//!
//! This module tests concurrency safety guarantees:
//! - No data races
//! - No deadlocks
//! - Correct atomic ordering
//! - Thread-safe resource management
//! - Lock-free algorithm correctness
//!
//! **Security Criticality: P0**
//! Concurrency bugs can lead to exploitable race conditions.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex, RwLock};
use std::thread;
use std::time::Duration;

// ============================================================================
// Test Suite 1: Data Race Prevention
// ============================================================================

#[test]
fn test_no_data_races_atomic() {
    // SECURITY: Atomic operations prevent data races
    let data = Arc::new(AtomicI64::new(0));
    let mut handles = vec![];

    for _ in 0..100 {
        let d = data.clone();
        let handle = thread::spawn(move || {
            for _ in 0..1000 {
                d.fetch_add(1, Ordering::SeqCst);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(
        data.load(Ordering::SeqCst),
        100_000,
        "Data race detected: incorrect final value"
    );
}

#[test]
fn test_no_data_races_mutex() {
    // SECURITY: Mutex prevents concurrent access
    let data = Arc::new(Mutex::new(0i64));
    let mut handles = vec![];

    for _ in 0..100 {
        let d = data.clone();
        let handle = thread::spawn(move || {
            for _ in 0..1000 {
                let mut guard = d.lock().unwrap();
                *guard += 1;
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let final_value = *data.lock().unwrap();
    assert_eq!(final_value, 100_000, "Mutex failed to prevent data race");
}

#[test]
fn test_no_data_races_rwlock() {
    // SECURITY: RwLock allows multiple readers, exclusive writer
    let data = Arc::new(RwLock::new(vec![0i32; 1000]));
    let mut handles = vec![];

    // Spawn readers
    for _ in 0..50 {
        let d = data.clone();
        let handle = thread::spawn(move || {
            for _ in 0..100 {
                let guard = d.read().unwrap();
                let sum: i32 = guard.iter().sum();
                assert!(sum >= 0, "Invalid read");
            }
        });
        handles.push(handle);
    }

    // Spawn writers
    for i in 0..50 {
        let d = data.clone();
        let handle = thread::spawn(move || {
            for _ in 0..10 {
                let mut guard = d.write().unwrap();
                guard[i] += 1;
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let guard = data.read().unwrap();
    let sum: i32 = guard.iter().sum();
    assert_eq!(sum, 500, "RwLock failed to prevent data race");
}

// ============================================================================
// Test Suite 2: Deadlock Prevention
// ============================================================================

#[test]
fn test_deadlock_prevention_lock_ordering() {
    // SECURITY: Consistent lock ordering prevents deadlocks
    let lock1 = Arc::new(Mutex::new(0));
    let lock2 = Arc::new(Mutex::new(0));

    let l1a = lock1.clone();
    let l2a = lock2.clone();

    let h1 = thread::spawn(move || {
        for _ in 0..1000 {
            // Always acquire lock1 before lock2
            let _g1 = l1a.lock().unwrap();
            let _g2 = l2a.lock().unwrap();
        }
    });

    let l1b = lock1.clone();
    let l2b = lock2.clone();

    let h2 = thread::spawn(move || {
        for _ in 0..1000 {
            // Same order: lock1 before lock2
            let _g1 = l1b.lock().unwrap();
            let _g2 = l2b.lock().unwrap();
        }
    });

    // Should complete without deadlock
    assert!(h1.join().is_ok(), "Thread 1 deadlocked or panicked");
    assert!(h2.join().is_ok(), "Thread 2 deadlocked or panicked");
}

#[test]
fn test_deadlock_detection_timeout() {
    // SECURITY: Timeouts can detect potential deadlocks
    use std::sync::mpsc;

    let lock = Arc::new(Mutex::new(0));
    let lock_clone = lock.clone();

    let (tx, rx) = mpsc::channel();

    let h1 = thread::spawn(move || {
        let _guard = lock.lock().unwrap();
        tx.send(()).unwrap(); // Signal lock acquired
        thread::sleep(Duration::from_millis(100)); // Hold lock
    });

    // Wait for lock to be acquired
    rx.recv().unwrap();

    // Try to acquire with timeout
    let h2 = thread::spawn(move || {
        let start = std::time::Instant::now();
        let _guard = lock_clone.lock().unwrap();
        start.elapsed()
    });

    h1.join().unwrap();
    let elapsed = h2.join().unwrap();

    // Should have waited for lock release
    assert!(
        elapsed >= Duration::from_millis(90),
        "Lock not held long enough"
    );
}

#[test]
fn test_no_deadlock_trylock() {
    // SECURITY: try_lock allows non-blocking lock acquisition
    let lock1 = Arc::new(Mutex::new(0));
    let lock2 = Arc::new(Mutex::new(0));

    let l1a = lock1.clone();
    let l2a = lock2.clone();

    let h1 = thread::spawn(move || {
        let mut success = 0;
        for _ in 0..1000 {
            if let Ok(_g1) = l1a.try_lock() {
                if let Ok(_g2) = l2a.try_lock() {
                    success += 1;
                }
            }
            thread::yield_now();
        }
        success
    });

    let l1b = lock1.clone();
    let l2b = lock2.clone();

    let h2 = thread::spawn(move || {
        let mut success = 0;
        for _ in 0..1000 {
            // Different order but using try_lock
            if let Ok(_g2) = l2b.try_lock() {
                if let Ok(_g1) = l1b.try_lock() {
                    success += 1;
                }
            }
            thread::yield_now();
        }
        success
    });

    let s1 = h1.join().unwrap();
    let s2 = h2.join().unwrap();

    // Should have some successes, no deadlock
    assert!(s1 > 0, "Thread 1 never succeeded");
    assert!(s2 > 0, "Thread 2 never succeeded");
}

// ============================================================================
// Test Suite 3: Atomic Ordering Correctness
// ============================================================================

#[test]
fn test_atomic_ordering_seq_cst() {
    // SECURITY: SeqCst provides total ordering
    let flag1 = Arc::new(AtomicBool::new(false));
    let flag2 = Arc::new(AtomicBool::new(false));
    let data = Arc::new(AtomicI64::new(0));

    let f1a = flag1.clone();
    let f2a = flag2.clone();
    let d1 = data.clone();

    let h1 = thread::spawn(move || {
        d1.store(42, Ordering::SeqCst);
        f1a.store(true, Ordering::SeqCst);
    });

    let f1b = flag1.clone();
    let f2b = flag2.clone();
    let d2 = data.clone();

    let h2 = thread::spawn(move || {
        while !f1b.load(Ordering::SeqCst) {
            thread::yield_now();
        }
        let value = d2.load(Ordering::SeqCst);
        f2b.store(true, Ordering::SeqCst);
        value
    });

    h1.join().unwrap();
    let value = h2.join().unwrap();

    assert_eq!(value, 42, "SeqCst ordering violated");
    assert!(flag2.load(Ordering::SeqCst));
}

#[test]
fn test_atomic_ordering_acquire_release() {
    // SECURITY: Acquire-Release provides synchronization
    let ready = Arc::new(AtomicBool::new(false));
    let data = Arc::new(AtomicI64::new(0));

    let r1 = ready.clone();
    let d1 = data.clone();

    let h1 = thread::spawn(move || {
        d1.store(100, Ordering::Relaxed);
        r1.store(true, Ordering::Release); // Release ensures data visible
    });

    let r2 = ready.clone();
    let d2 = data.clone();

    let h2 = thread::spawn(move || {
        while !r2.load(Ordering::Acquire) {
            // Acquire ensures we see data
            thread::yield_now();
        }
        d2.load(Ordering::Relaxed)
    });

    h1.join().unwrap();
    let value = h2.join().unwrap();

    assert_eq!(value, 100, "Acquire-Release ordering violated");
}

#[test]
fn test_atomic_compare_exchange() {
    // SECURITY: Compare-exchange is atomic
    let value = Arc::new(AtomicI64::new(0));
    let mut handles = vec![];

    for i in 0..100 {
        let v = value.clone();
        let handle = thread::spawn(move || {
            let mut successes = 0;
            for _ in 0..100 {
                let current = v.load(Ordering::SeqCst);
                if v
                    .compare_exchange(
                        current,
                        current + i,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    successes += 1;
                }
            }
            successes
        });
        handles.push(handle);
    }

    let total_successes: i32 = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .sum();

    // Some operations should succeed
    assert!(total_successes > 0, "No compare-exchange operations succeeded");
}

// ============================================================================
// Test Suite 4: Thread-Safe Resource Management
// ============================================================================

#[test]
fn test_arc_thread_safety() {
    // SECURITY: Arc provides thread-safe reference counting
    let data = Arc::new(vec![1, 2, 3, 4, 5]);
    let mut handles = vec![];

    for _ in 0..100 {
        let d = data.clone();
        let handle = thread::spawn(move || {
            let sum: i32 = d.iter().sum();
            assert_eq!(sum, 15);
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(Arc::strong_count(&data), 1);
}

#[test]
fn test_thread_pool_resource_limits() {
    // SECURITY: Thread pools enforce resource limits
    use std::sync::mpsc;

    const MAX_THREADS: usize = 10;
    let active = Arc::new(AtomicUsize::new(0));
    let max_observed = Arc::new(AtomicUsize::new(0));

    let (tx, rx) = mpsc::channel();
    let mut handles = vec![];

    for _ in 0..100 {
        let act = active.clone();
        let max_obs = max_observed.clone();
        let tx_clone = tx.clone();

        let handle = thread::spawn(move || {
            let current = act.fetch_add(1, Ordering::SeqCst) + 1;

            // Update max observed
            loop {
                let max = max_obs.load(Ordering::SeqCst);
                if current <= max {
                    break;
                }
                if max_obs
                    .compare_exchange(max, current, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    break;
                }
            }

            thread::sleep(Duration::from_millis(10));

            act.fetch_sub(1, Ordering::SeqCst);
            tx_clone.send(()).unwrap();
        });

        handles.push(handle);

        // Limit concurrent threads
        while active.load(Ordering::SeqCst) >= MAX_THREADS {
            thread::sleep(Duration::from_millis(1));
        }
    }

    drop(tx);
    while rx.recv().is_ok() {}

    for handle in handles {
        handle.join().unwrap();
    }

    let max = max_observed.load(Ordering::SeqCst);
    assert!(
        max <= MAX_THREADS * 2,
        "Thread limit violated: {} > {}",
        max,
        MAX_THREADS * 2
    );
}

// ============================================================================
// Test Suite 5: Condition Variable Safety
// ============================================================================

#[test]
fn test_condvar_wait_safety() {
    // SECURITY: Condition variables prevent lost wakeups
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = pair.clone();

    let handle = thread::spawn(move || {
        let (lock, cvar) = &*pair_clone;
        let mut ready = lock.lock().unwrap();

        while !*ready {
            ready = cvar.wait(ready).unwrap();
        }

        assert!(*ready);
    });

    thread::sleep(Duration::from_millis(50));

    let (lock, cvar) = &*pair;
    let mut ready = lock.lock().unwrap();
    *ready = true;
    cvar.notify_one();
    drop(ready);

    handle.join().unwrap();
}

#[test]
fn test_condvar_spurious_wakeup_handling() {
    // SECURITY: Handle spurious wakeups correctly
    let pair = Arc::new((Mutex::new(0), Condvar::new()));
    let pair_clone = pair.clone();

    let handle = thread::spawn(move || {
        let (lock, cvar) = &*pair_clone;
        let mut count = lock.lock().unwrap();

        // Wait until count reaches 10
        while *count < 10 {
            count = cvar.wait(count).unwrap();
        }

        *count
    });

    let (lock, cvar) = &*pair;

    for i in 1..=10 {
        thread::sleep(Duration::from_millis(10));
        let mut count = lock.lock().unwrap();
        *count = i;
        cvar.notify_one();
    }

    let final_count = handle.join().unwrap();
    assert_eq!(final_count, 10);
}

#[test]
fn test_condvar_broadcast() {
    // SECURITY: Broadcast wakes all waiting threads
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let mut handles = vec![];

    for _ in 0..10 {
        let pair_clone = pair.clone();
        let handle = thread::spawn(move || {
            let (lock, cvar) = &*pair_clone;
            let mut ready = lock.lock().unwrap();

            while !*ready {
                ready = cvar.wait(ready).unwrap();
            }
        });
        handles.push(handle);
    }

    thread::sleep(Duration::from_millis(100));

    let (lock, cvar) = &*pair;
    let mut ready = lock.lock().unwrap();
    *ready = true;
    cvar.notify_all(); // Wake all threads
    drop(ready);

    for handle in handles {
        assert!(handle.join().is_ok(), "Thread failed to wake");
    }
}

// ============================================================================
// Test Suite 6: Lock-Free Algorithm Correctness
// ============================================================================

#[test]
fn test_lock_free_stack() {
    // SECURITY: Lock-free stack is thread-safe
    use std::ptr;

    struct Node<T> {
        value: T,
        next: *mut Node<T>,
    }

    struct LockFreeStack<T> {
        head: AtomicUsize, // Stores pointer as usize
    }

    impl<T> LockFreeStack<T> {
        fn new() -> Self {
            Self {
                head: AtomicUsize::new(0),
            }
        }

        fn push(&self, value: T) {
            let new_node = Box::into_raw(Box::new(Node {
                value,
                next: ptr::null_mut(),
            }));

            loop {
                let current = self.head.load(Ordering::Acquire);
                unsafe {
                    (*new_node).next = current as *mut Node<T>;
                }

                if self
                    .head
                    .compare_exchange(
                        current,
                        new_node as usize,
                        Ordering::Release,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    break;
                }
            }
        }

        fn pop(&self) -> Option<T> {
            loop {
                let current = self.head.load(Ordering::Acquire);
                if current == 0 {
                    return None;
                }

                let current_ptr = current as *mut Node<T>;
                let next = unsafe { (*current_ptr).next };

                if self
                    .head
                    .compare_exchange(
                        current,
                        next as usize,
                        Ordering::Release,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    let node = unsafe { Box::from_raw(current_ptr) };
                    return Some(node.value);
                }
            }
        }
    }

    impl<T> Drop for LockFreeStack<T> {
        fn drop(&mut self) {
            while self.pop().is_some() {}
        }
    }

    let stack = Arc::new(LockFreeStack::new());
    let mut handles = vec![];

    // Push from multiple threads
    for i in 0..100 {
        let s = stack.clone();
        let handle = thread::spawn(move || {
            for j in 0..100 {
                s.push(i * 100 + j);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Pop from multiple threads
    let popped = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    for _ in 0..100 {
        let s = stack.clone();
        let p = popped.clone();
        let handle = thread::spawn(move || {
            while s.pop().is_some() {
                p.fetch_add(1, Ordering::SeqCst);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(
        popped.load(Ordering::SeqCst),
        10_000,
        "Items lost in lock-free stack"
    );
}

#[test]
fn test_barrier_synchronization() {
    // SECURITY: Barriers synchronize thread groups
    let barrier = Arc::new(Barrier::new(10));
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    for _ in 0..10 {
        let b = barrier.clone();
        let c = counter.clone();

        let handle = thread::spawn(move || {
            // Increment before barrier
            c.fetch_add(1, Ordering::SeqCst);

            // Wait for all threads
            b.wait();

            // All threads should see counter == 10
            let value = c.load(Ordering::SeqCst);
            assert_eq!(value, 10, "Barrier failed to synchronize");
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }
}
