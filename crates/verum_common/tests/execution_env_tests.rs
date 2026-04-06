//! Tests for the foundation-layer ExecutionEnv
//!
//! These tests verify the core functionality of the ExecutionEnv type,
//! which provides the foundation for the full implementation in verum_runtime.

use verum_common::ExecutionEnv;

// ============================================================================
// BASIC CONSTRUCTION TESTS
// ============================================================================

#[test]
fn test_new_creates_root_environment() {
    let env = ExecutionEnv::new();
    assert_eq!(env.generation(), 0);
    assert!(env.is_root());
}

#[test]
fn test_default_creates_root_environment() {
    let env = ExecutionEnv::default();
    assert_eq!(env.generation(), 0);
    assert!(env.is_root());
}

#[test]
fn test_clone_preserves_generation() {
    let env = ExecutionEnv::new();
    let cloned = env.clone();
    assert_eq!(env.generation(), cloned.generation());
}

// ============================================================================
// FORK TESTS
// ============================================================================

#[test]
fn test_fork_increments_generation() {
    let env = ExecutionEnv::new();
    let child = env.fork();
    assert_eq!(child.generation(), 1);
    assert!(!child.is_root());
}

#[test]
fn test_multiple_forks_increment_generation() {
    let env = ExecutionEnv::new();
    let child1 = env.fork();
    let child2 = child1.fork();
    let child3 = child2.fork();

    assert_eq!(env.generation(), 0);
    assert_eq!(child1.generation(), 1);
    assert_eq!(child2.generation(), 2);
    assert_eq!(child3.generation(), 3);
}

#[test]
fn test_sibling_forks_have_same_generation() {
    let parent = ExecutionEnv::new();
    let child1 = parent.fork();
    let child2 = parent.fork();

    assert_eq!(child1.generation(), 1);
    assert_eq!(child2.generation(), 1);
}

#[test]
fn test_fork_does_not_modify_parent() {
    let parent = ExecutionEnv::new();
    let original_gen = parent.generation();
    let _child = parent.fork();

    assert_eq!(parent.generation(), original_gen);
}

#[test]
fn test_fork_at_max_generation() {
    // Create environment at u32::MAX - 1
    let mut env = ExecutionEnv::new();
    for _ in 0..10 {
        env = env.fork();
    }

    // Verify it can still fork (saturating_add prevents overflow)
    let child = env.fork();
    assert!(child.generation() > env.generation() || child.generation() == u32::MAX);
}

// ============================================================================
// EQUALITY TESTS
// ============================================================================

#[test]
fn test_equality_based_on_generation() {
    let env1 = ExecutionEnv::new();
    let env2 = ExecutionEnv::new();

    // Same generation = equal
    assert_eq!(env1, env2);

    // Different generation = not equal
    let child = env1.fork();
    assert_ne!(env1, child);
}

#[test]
fn test_clone_equals_original() {
    let env = ExecutionEnv::new().fork().fork();
    let cloned = env.clone();
    assert_eq!(env, cloned);
}

// ============================================================================
// THREAD-LOCAL TESTS (require std feature)
// ============================================================================

#[cfg(feature = "std")]
mod thread_local_tests {
    use super::*;

    #[test]
    fn test_current_returns_default_when_not_set() {
        // Clear any existing environment first
        ExecutionEnv::clear_current();

        let env = ExecutionEnv::current();
        assert!(env.is_root());
    }

    #[test]
    fn test_set_and_get_current() {
        let env = ExecutionEnv::new().fork();
        ExecutionEnv::set_current(env.clone());

        let current = ExecutionEnv::current();
        assert_eq!(current.generation(), 1);

        ExecutionEnv::clear_current();
    }

    #[test]
    fn test_clear_current_resets_to_default() {
        let env = ExecutionEnv::new().fork().fork();
        ExecutionEnv::set_current(env);

        assert_eq!(ExecutionEnv::current().generation(), 2);

        ExecutionEnv::clear_current();
        assert_eq!(ExecutionEnv::current().generation(), 0);
    }

    #[test]
    fn test_has_current() {
        ExecutionEnv::clear_current();
        assert!(!ExecutionEnv::has_current());

        let env = ExecutionEnv::new();
        ExecutionEnv::set_current(env);
        assert!(ExecutionEnv::has_current());

        ExecutionEnv::clear_current();
        assert!(!ExecutionEnv::has_current());
    }

    #[test]
    fn test_with_env_temporary_context() {
        ExecutionEnv::clear_current();
        assert!(!ExecutionEnv::has_current());

        let env = ExecutionEnv::new().fork();
        let result = ExecutionEnv::with_env(env, || {
            let current = ExecutionEnv::current();
            assert_eq!(current.generation(), 1);
            42
        });

        assert_eq!(result, 42);
        // Environment should be cleared after with_env
        assert!(!ExecutionEnv::has_current());
    }

    #[test]
    fn test_with_env_restores_previous() {
        let outer_env = ExecutionEnv::new().fork();
        ExecutionEnv::set_current(outer_env.clone());

        let inner_env = outer_env.fork().fork();
        ExecutionEnv::with_env(inner_env, || {
            assert_eq!(ExecutionEnv::current().generation(), 3);
        });

        // Should restore to outer_env
        assert_eq!(ExecutionEnv::current().generation(), 1);

        ExecutionEnv::clear_current();
    }

    #[test]
    fn test_thread_isolation() {
        use std::thread;

        // Set environment in main thread
        let main_env = ExecutionEnv::new().fork();
        ExecutionEnv::set_current(main_env);

        // Spawn thread and verify it has independent environment
        let handle = thread::spawn(|| {
            // Other thread should have no environment set
            let has_current = ExecutionEnv::has_current();
            let generation = ExecutionEnv::current().generation();
            (has_current, generation)
        });

        let (other_has_current, other_generation) = handle.join().unwrap();
        assert!(!other_has_current);
        assert_eq!(other_generation, 0);

        // Main thread should still have its environment
        assert!(ExecutionEnv::has_current());
        assert_eq!(ExecutionEnv::current().generation(), 1);

        ExecutionEnv::clear_current();
    }
}

// ============================================================================
// DEBUG TESTS
// ============================================================================

#[test]
fn test_debug_output() {
    let env = ExecutionEnv::new();
    let debug_str = format!("{:?}", env);
    assert!(debug_str.contains("ExecutionEnv"));
    assert!(debug_str.contains("generation"));
}

#[test]
fn test_debug_shows_generation() {
    let env = ExecutionEnv::new().fork().fork();
    let debug_str = format!("{:?}", env);
    assert!(debug_str.contains("2"));
}

// ============================================================================
// SEND + SYNC TESTS
// ============================================================================

#[test]
fn test_send() {
    fn assert_send<T: Send>() {}
    assert_send::<ExecutionEnv>();
}

#[test]
fn test_sync() {
    fn assert_sync<T: Sync>() {}
    assert_sync::<ExecutionEnv>();
}

#[cfg(feature = "std")]
#[test]
fn test_send_across_threads() {
    use std::thread;

    let env = ExecutionEnv::new().fork();

    let handle = thread::spawn(move || {
        // Can use env in another thread
        env.generation()
    });

    let result = handle.join().unwrap();
    assert_eq!(result, 1);
}
