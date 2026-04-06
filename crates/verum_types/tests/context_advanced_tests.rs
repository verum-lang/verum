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
//! Advanced context system tests for type checking
//!
//! Tests cover:
//! - Context set operations (union, subset)
//! - Context checker scoping
//! - Context provide blocks
//! - Context requirement propagation through call chains
//! - Sub-context validation
//! - Context environment hierarchy

use verum_ast::decl::{ContextDecl, Visibility};
use verum_ast::span::Span;
use verum_ast::ty::Ident;
use verum_common::Text;
use verum_types::context_check::*;
use verum_types::TypeError;

/// Helper to create a simple context declaration
fn make_context(name: &str) -> ContextDecl {
    ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new(name.to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: vec![].into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    }
}

fn make_context_with_subs(name: &str, subs: Vec<&str>) -> ContextDecl {
    let sub_contexts: Vec<ContextDecl> = subs
        .iter()
        .map(|s| make_context(s))
        .collect();
    ContextDecl {
        visibility: Visibility::Public,
        name: Ident::new(name.to_string(), Span::default()),
        methods: vec![].into(),
        sub_contexts: sub_contexts.into(),
        associated_types: vec![].into(),
        associated_consts: vec![].into(),
        span: Span::default(),
        generics: vec![].into(),
        is_async: false,
    }
}

fn make_req(name: &str) -> ContextRequirement {
    ContextRequirement::new(name.to_string(), Span::default())
}

fn make_sub_req(context: &str, sub: &str) -> ContextRequirement {
    ContextRequirement::with_sub(context.to_string(), sub.to_string(), Span::default())
}

// ============================================================================
// Context Set Operations
// ============================================================================

#[test]
fn test_context_set_union_basic() {
    let mut set1 = ContextSet::new();
    set1.add(make_req("Database"));

    let mut set2 = ContextSet::new();
    set2.add(make_req("Logger"));

    let union = set1.union(&set2);
    assert_eq!(union.len(), 2);
    assert!(union.contains("Database"));
    assert!(union.contains("Logger"));
}

#[test]
fn test_context_set_union_overlapping() {
    let mut set1 = ContextSet::new();
    set1.add(make_req("Database"));
    set1.add(make_req("Logger"));

    let mut set2 = ContextSet::new();
    set2.add(make_req("Logger"));
    set2.add(make_req("Cache"));

    let union = set1.union(&set2);
    assert_eq!(union.len(), 3);
    assert!(union.contains("Database"));
    assert!(union.contains("Logger"));
    assert!(union.contains("Cache"));
}

#[test]
fn test_context_set_empty_operations() {
    let empty = ContextSet::new();
    let mut non_empty = ContextSet::new();
    non_empty.add(make_req("Database"));

    // Union with empty
    let union = empty.union(&non_empty);
    assert_eq!(union.len(), 1);

    // Empty is subset of everything
    assert!(empty.is_subset(&non_empty));
    assert!(empty.is_subset(&empty));
}

#[test]
fn test_context_set_duplicate_add() {
    let mut set = ContextSet::new();
    set.add(make_req("Database"));
    set.add(make_req("Database"));
    assert_eq!(set.len(), 1, "Duplicate adds should not increase size");
}

#[test]
fn test_context_set_subset_proper() {
    let mut small = ContextSet::new();
    small.add(make_req("Database"));

    let mut big = ContextSet::new();
    big.add(make_req("Database"));
    big.add(make_req("Logger"));
    big.add(make_req("Cache"));

    assert!(small.is_subset(&big));
    assert!(!big.is_subset(&small));
}

#[test]
fn test_context_set_self_subset() {
    let mut set = ContextSet::new();
    set.add(make_req("Database"));
    set.add(make_req("Logger"));

    assert!(set.is_subset(&set));
}

#[test]
fn test_context_set_with_sub_contexts() {
    let mut set = ContextSet::new();
    set.add(make_sub_req("FileSystem", "Read"));
    set.add(make_sub_req("FileSystem", "Write"));

    assert_eq!(set.len(), 2);
    // Sub-context paths use contains_full_path for dotted names
    assert!(set.contains_full_path("FileSystem.Read"));
    assert!(set.contains_full_path("FileSystem.Write"));
}

// ============================================================================
// Context Checker Registration and Lookup
// ============================================================================

#[test]
fn test_checker_register_and_check() {
    let mut checker = ContextChecker::new();
    checker.register_context("Database".to_string(), make_context("Database"));
    checker.register_context("Logger".to_string(), make_context("Logger"));

    // After registration, contexts should be known (for sub-context checks)
    let result = checker.check_sub_context("Database", "Nonexistent", Span::default());
    // Should not be UndefinedContext since it's registered
    match result {
        Err(TypeError::UndefinedContext { .. }) => {
            panic!("Database should be registered, not undefined");
        }
        _ => {
            // Any other error is expected (InvalidSubContext, NoSubContexts, etc.)
        }
    }
}

#[test]
fn test_checker_unregistered_context() {
    let checker = ContextChecker::new();
    let result = checker.check_sub_context("Nonexistent", "Read", Span::default());
    match result {
        Err(TypeError::UndefinedContext { name, .. }) => {
            assert_eq!(name, "Nonexistent");
        }
        _ => panic!("Expected UndefinedContext error"),
    }
}

// ============================================================================
// Context Checker Scope Management
// ============================================================================

#[test]
fn test_checker_nested_scope() {
    let mut checker = ContextChecker::new();

    let mut contexts = ContextSet::new();
    contexts.add(make_req("Database"));
    checker.set_required(contexts);

    assert!(checker.is_available("Database"));
    assert!(!checker.is_available("Logger"));

    checker.enter_scope();
    // Database still available from outer scope
    assert!(checker.is_available("Database"));
    checker.exit_scope();

    // Still available after exiting inner scope
    assert!(checker.is_available("Database"));
}

#[test]
fn test_checker_scope_enter_exit_empty() {
    let mut checker = ContextChecker::new();
    checker.enter_scope();
    checker.enter_scope();
    checker.exit_scope();
    checker.exit_scope();
    // No panic = success
}

// ============================================================================
// Context Propagation
// ============================================================================

#[test]
fn test_propagation_exact_match() {
    let mut checker = ContextChecker::new();

    let mut contexts = ContextSet::new();
    contexts.add(make_req("Database"));
    contexts.add(make_req("Logger"));
    checker.set_required(contexts);

    // Callee needs exactly Database and Logger
    let mut callee = ContextSet::new();
    callee.add(make_req("Database"));
    callee.add(make_req("Logger"));

    let result = checker.check_call_propagation(&callee, Span::default());
    assert!(result.is_ok(), "Exact match should succeed");
}

#[test]
fn test_propagation_superset_ok() {
    let mut checker = ContextChecker::new();

    let mut contexts = ContextSet::new();
    contexts.add(make_req("Database"));
    contexts.add(make_req("Logger"));
    contexts.add(make_req("Cache"));
    checker.set_required(contexts);

    // Callee only needs Database
    let mut callee = ContextSet::new();
    callee.add(make_req("Database"));

    let result = checker.check_call_propagation(&callee, Span::default());
    assert!(result.is_ok(), "Superset should succeed");
}

#[test]
fn test_propagation_missing_context() {
    let mut checker = ContextChecker::new();

    let mut contexts = ContextSet::new();
    contexts.add(make_req("Database"));
    checker.set_required(contexts);

    // Callee needs Database AND Logger
    let mut callee = ContextSet::new();
    callee.add(make_req("Database"));
    callee.add(make_req("Logger"));

    let result = checker.check_call_propagation(&callee, Span::default());
    assert!(result.is_err(), "Missing Logger should fail");
}

#[test]
fn test_propagation_empty_callee() {
    let checker = ContextChecker::new();
    let callee = ContextSet::new();

    let result = checker.check_call_propagation(&callee, Span::default());
    assert!(result.is_ok(), "Empty callee requirements should always succeed");
}

// ============================================================================
// Context Call Checking
// ============================================================================

#[test]
fn test_missing_context_error() {
    let checker = ContextChecker::new();

    let result = checker.check_context_call("Database", "query", Span::default());
    match result {
        Err(TypeError::MissingContext { context, .. }) => {
            assert_eq!(context, "Database");
        }
        _ => panic!("Expected MissingContext error"),
    }
}

#[test]
fn test_multiple_missing_context_errors() {
    let checker = ContextChecker::new();

    assert!(checker.check_context_call("Database", "query", Span::default()).is_err());
    assert!(checker.check_context_call("Logger", "log", Span::default()).is_err());
    assert!(checker.check_context_call("Cache", "get", Span::default()).is_err());
}

#[test]
fn test_context_available_after_set_required() {
    let mut checker = ContextChecker::new();

    assert!(!checker.is_available("Database"));

    let mut contexts = ContextSet::new();
    contexts.add(make_req("Database"));
    contexts.add(make_req("Logger"));
    checker.set_required(contexts);

    assert!(checker.is_available("Database"));
    assert!(checker.is_available("Logger"));
    assert!(!checker.is_available("Cache"));
}

// ============================================================================
// Sub-Context Validation
// ============================================================================

#[test]
fn test_valid_sub_context_check() {
    let mut checker = ContextChecker::new();
    let fs = make_context_with_subs("FileSystem", vec!["Read", "Write", "Admin"]);
    checker.register_context("FileSystem".to_string(), fs);

    assert!(checker.check_sub_context("FileSystem", "Read", Span::default()).is_ok());
    assert!(checker.check_sub_context("FileSystem", "Write", Span::default()).is_ok());
    assert!(checker.check_sub_context("FileSystem", "Admin", Span::default()).is_ok());
}

#[test]
fn test_invalid_sub_context_check() {
    let mut checker = ContextChecker::new();
    let fs = make_context_with_subs("FileSystem", vec!["Read", "Write"]);
    checker.register_context("FileSystem".to_string(), fs);

    let result = checker.check_sub_context("FileSystem", "Execute", Span::default());
    match result {
        Err(TypeError::InvalidSubContext { context, sub_context, .. }) => {
            assert_eq!(context, "FileSystem");
            assert_eq!(sub_context, "Execute");
        }
        _ => panic!("Expected InvalidSubContext error"),
    }
}

// ============================================================================
// Context Environment
// ============================================================================

#[test]
fn test_context_env_empty() {
    let env = ContextEnv::new();
    assert!(!env.has_context("Database"));
    assert_eq!(env.all_contexts().len(), 0);
}

#[test]
fn test_context_env_child() {
    let env = ContextEnv::new();
    let child = env.child();
    assert!(!child.has_context("Database"));
    assert_eq!(child.all_contexts().len(), 0);
}

#[test]
fn test_context_env_nested_children() {
    let env = ContextEnv::new();
    let child = env.child();
    let grandchild = child.child();

    assert_eq!(env.all_contexts().len(), 0);
    assert_eq!(child.all_contexts().len(), 0);
    assert_eq!(grandchild.all_contexts().len(), 0);
}

// ============================================================================
// Context Requirement
// ============================================================================

#[test]
fn test_requirement_full_path() {
    let simple = make_req("Database");
    assert_eq!(simple.full_path(), "Database");

    let sub = make_sub_req("FileSystem", "Read");
    assert_eq!(sub.full_path(), "FileSystem.Read");
}

#[test]
fn test_requirement_equality_by_path() {
    let req1 = make_req("Database");
    let req2 = make_req("Database");
    assert_eq!(req1.full_path(), req2.full_path());
}

#[test]
fn test_requirement_sub_vs_plain_different() {
    let plain = make_req("FileSystem");
    let sub = make_sub_req("FileSystem", "Read");
    assert_ne!(plain.full_path(), sub.full_path());
}
