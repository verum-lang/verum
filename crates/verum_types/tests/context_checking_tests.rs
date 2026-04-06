#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for Context System type checking
//
// Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration

use verum_ast::span::Span;
use verum_types::{
    ContextChecker, ContextSet, TwoLevelContextEnv as ContextEnv,
    TwoLevelContextRequirement as ContextRequirement, TypeError,
};

#[test]
fn test_context_requirement_creation() {
    let req = ContextRequirement::new("Database".to_string(), Span::default());
    assert_eq!(req.name, "Database");
    assert_eq!(req.full_path(), "Database");
}

#[test]
fn test_context_requirement_with_sub() {
    let req = ContextRequirement::with_sub(
        "FileSystem".to_string(),
        "Read".to_string(),
        Span::default(),
    );
    assert_eq!(req.name, "FileSystem");
    assert_eq!(req.full_path(), "FileSystem.Read");
}

#[test]
fn test_context_set_operations() {
    let mut set = ContextSet::new();
    assert!(set.is_empty());

    let req1 = ContextRequirement::new("Database".to_string(), Span::default());
    set.add(req1);
    assert_eq!(set.len(), 1);
    assert!(set.contains("Database"));

    let req2 = ContextRequirement::new("Logger".to_string(), Span::default());
    let set2 = ContextSet::singleton(req2);

    let union = set.union(&set2);
    assert_eq!(union.len(), 2);
    assert!(union.contains("Database"));
    assert!(union.contains("Logger"));
}

#[test]
fn test_context_set_subset() {
    let req1 = ContextRequirement::new("Database".to_string(), Span::default());
    let req2 = ContextRequirement::new("Logger".to_string(), Span::default());

    let set1 = ContextSet::singleton(req1.clone());
    let mut set2 = ContextSet::new();
    set2.add(req1);
    set2.add(req2);

    assert!(
        set1.is_subset(&set2),
        "Single context should be subset of larger set"
    );
    assert!(
        !set2.is_subset(&set1),
        "Larger set should not be subset of single context"
    );
}

#[test]
fn test_context_env_scoping() {
    let env = ContextEnv::new();
    assert!(!env.has_context("Logger"));

    // Create child scope
    let child = env.child();
    assert!(!child.has_context("Logger"));

    // Test that all_contexts works
    assert_eq!(env.all_contexts().len(), 0);
}

#[test]
fn test_context_checker_basic() {
    let mut checker = ContextChecker::new();

    // Set required contexts
    let req = ContextRequirement::new("Database".to_string(), Span::default());
    let mut contexts = ContextSet::new();
    contexts.add(req);
    checker.set_required(contexts);

    // Check that Database is available (it's required)
    assert!(checker.is_available("Database"));
    assert!(!checker.is_available("Logger"));
}

#[test]
fn test_context_checker_scoping() {
    let mut checker = ContextChecker::new();

    // Enter scope
    checker.enter_scope();

    // Exit scope
    checker.exit_scope();
}

#[test]
fn test_context_call_missing_context() {
    let checker = ContextChecker::new();

    // Try to call Logger.log without having Logger available
    let result = checker.check_context_call("Logger", "log", Span::default());
    assert!(result.is_err(), "Should fail when context is not available");

    match result {
        Err(TypeError::MissingContext { context, .. }) => {
            assert_eq!(context, "Logger");
        }
        _ => panic!("Expected MissingContext error"),
    }
}

#[test]
fn test_context_propagation_missing() {
    let checker = ContextChecker::new();

    // Create a context set that callee requires
    let req = ContextRequirement::new("Database".to_string(), Span::default());
    let mut callee_contexts = ContextSet::new();
    callee_contexts.add(req);

    // Check propagation (should fail because Database is not available)
    let result = checker.check_call_propagation(&callee_contexts, Span::default());
    assert!(
        result.is_err(),
        "Should fail when required context is not propagated"
    );
}

#[test]
fn test_context_propagation_success() {
    let mut checker = ContextChecker::new();

    // Set required contexts
    let req = ContextRequirement::new("Database".to_string(), Span::default());
    let mut contexts = ContextSet::new();
    contexts.add(req.clone());
    checker.set_required(contexts);

    // Create callee context set
    let mut callee_contexts = ContextSet::new();
    callee_contexts.add(req);

    // Check propagation (should succeed)
    let result = checker.check_call_propagation(&callee_contexts, Span::default());
    assert!(
        result.is_ok(),
        "Should succeed when required context is available"
    );
}

#[test]
fn test_context_set_from_iterator() {
    let req1 = ContextRequirement::new("Database".to_string(), Span::default());
    let req2 = ContextRequirement::new("Logger".to_string(), Span::default());

    let set: ContextSet = vec![req1, req2].into_iter().collect();
    assert_eq!(set.len(), 2);
    assert!(set.contains("Database"));
    assert!(set.contains("Logger"));
}

#[test]
fn test_sub_context_path_matching() {
    let req = ContextRequirement::with_sub(
        "FileSystem".to_string(),
        "Read".to_string(),
        Span::default(),
    );

    let set = ContextSet::singleton(req);
    assert!(set.contains_full_path("FileSystem.Read"));
    assert!(!set.contains_full_path("FileSystem"));
    assert!(!set.contains_full_path("FileSystem.Write"));
}
