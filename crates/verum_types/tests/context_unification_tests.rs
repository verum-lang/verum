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
//! Tests for context system: ContextExpr, ContextRequirement, ContextRef,
//! and context unification through the type system.
//!
//! These tests verify the real API of the context system as exposed by
//! verum_types::di::requirement and verum_types::unify.

use std::any::TypeId;
use verum_ast::span::Span;
use verum_common::{List, Text};
use verum_types::di::requirement::{ContextExpr, ContextRef, ContextRequirement};
use verum_types::ty::{Type, TypeVar};
use verum_types::unify::Unifier;

fn make_span() -> Span {
    Span::dummy()
}

fn make_logger_ref() -> ContextRef {
    ContextRef::new("Logger".into(), TypeId::of::<()>())
}

fn make_db_ref() -> ContextRef {
    ContextRef::new("Database".into(), TypeId::of::<String>())
}

fn make_logger_req() -> ContextRequirement {
    ContextRequirement::single(make_logger_ref())
}

fn make_db_req() -> ContextRequirement {
    ContextRequirement::single(make_db_ref())
}

fn make_logger_db_req() -> ContextRequirement {
    ContextRequirement::from_contexts(vec![make_logger_ref(), make_db_ref()])
}

// ============================================================================
// ContextRequirement basic operations
// ============================================================================

#[test]
fn test_context_requirement_empty() {
    let req = ContextRequirement::empty();
    assert!(req.is_empty());
    assert_eq!(req.len(), 0);
}

#[test]
fn test_context_requirement_single() {
    let req = make_logger_req();
    assert!(!req.is_empty());
    assert_eq!(req.len(), 1);
    assert!(req.requires("Logger"));
    assert!(!req.requires("Database"));
}

#[test]
fn test_context_requirement_multiple() {
    let req = make_logger_db_req();
    assert_eq!(req.len(), 2);
    assert!(req.requires("Logger"));
    assert!(req.requires("Database"));
    assert!(!req.requires("FileSystem"));
}

#[test]
fn test_context_requirement_add_context() {
    let mut req = ContextRequirement::empty();
    req.add_context(make_logger_ref());
    assert_eq!(req.len(), 1);
    assert!(req.requires("Logger"));

    req.add_context(make_db_ref());
    assert_eq!(req.len(), 2);
    assert!(req.requires("Database"));
}

#[test]
fn test_context_requirement_remove_context() {
    let mut req = make_logger_db_req();
    assert_eq!(req.len(), 2);

    let removed = req.remove_context("Logger");
    assert!(removed);
    assert_eq!(req.len(), 1);
    assert!(!req.requires("Logger"));
    assert!(req.requires("Database"));
}

#[test]
fn test_context_requirement_remove_nonexistent() {
    let mut req = make_logger_req();
    let removed = req.remove_context("NonExistent");
    assert!(!removed);
    assert_eq!(req.len(), 1);
}

#[test]
fn test_context_requirement_get_context() {
    let req = make_logger_db_req();
    let logger = req.get_context("Logger");
    assert!(logger.is_some());
    assert_eq!(logger.unwrap().name.as_str(), "Logger");

    let missing = req.get_context("FileSystem");
    assert!(missing.is_none());
}

// ============================================================================
// ContextRequirement merging
// ============================================================================

#[test]
fn test_context_requirement_merge_disjoint() {
    let req1 = make_logger_req();
    let req2 = make_db_req();
    let merged = req1.merge(&req2);
    assert_eq!(merged.len(), 2);
    assert!(merged.requires("Logger"));
    assert!(merged.requires("Database"));
}

#[test]
fn test_context_requirement_merge_overlapping() {
    let req1 = make_logger_db_req();
    let req2 = make_logger_req();
    let merged = req1.merge(&req2);
    // Logger should not be duplicated
    assert_eq!(merged.len(), 2);
}

#[test]
fn test_context_requirement_merge_empty() {
    let req1 = make_logger_req();
    let empty = ContextRequirement::empty();
    let merged = req1.merge(&empty);
    assert_eq!(merged.len(), 1);
    assert!(merged.requires("Logger"));
}

// ============================================================================
// ContextRequirement subset checking
// ============================================================================

#[test]
fn test_context_requirement_is_subset() {
    let small = make_logger_req();
    let big = make_logger_db_req();
    assert!(small.is_subset_of(&big));
    assert!(!big.is_subset_of(&small));
}

#[test]
fn test_context_requirement_is_subset_self() {
    let req = make_logger_db_req();
    assert!(req.is_subset_of(&req));
}

#[test]
fn test_context_requirement_empty_is_subset_of_all() {
    let empty = ContextRequirement::empty();
    let any = make_logger_db_req();
    assert!(empty.is_subset_of(&any));
    assert!(empty.is_subset_of(&empty));
}

// ============================================================================
// ContextExpr: concrete vs variable
// ============================================================================

#[test]
fn test_context_expr_concrete() {
    let expr = ContextExpr::concrete(make_logger_req());
    assert!(expr.is_concrete());
    assert!(!expr.is_variable());
    assert!(!expr.is_empty());
    assert_eq!(expr.len(), 1);
}

#[test]
fn test_context_expr_variable() {
    let var = TypeVar::fresh();
    let expr = ContextExpr::variable(var);
    assert!(expr.is_variable());
    assert!(!expr.is_concrete());
    assert!(!expr.is_empty()); // Variables may bind to non-empty
    assert_eq!(expr.len(), 0); // Length is 0 for variables (unknown)
}

#[test]
fn test_context_expr_empty() {
    let expr = ContextExpr::empty();
    assert!(expr.is_concrete());
    assert!(expr.is_empty());
    assert_eq!(expr.len(), 0);
}

#[test]
fn test_context_expr_as_variable() {
    let var = TypeVar::fresh();
    let expr = ContextExpr::variable(var);
    assert_eq!(expr.as_variable(), Some(var));

    let concrete = ContextExpr::concrete(make_logger_req());
    assert_eq!(concrete.as_variable(), None);
}

#[test]
fn test_context_expr_as_concrete() {
    let req = make_logger_req();
    let expr = ContextExpr::concrete(req.clone());
    assert!(expr.as_concrete().is_some());

    let var_expr = ContextExpr::variable(TypeVar::fresh());
    assert!(var_expr.as_concrete().is_none());
}

#[test]
fn test_context_expr_requires() {
    let expr = ContextExpr::concrete(make_logger_db_req());
    assert!(expr.requires("Logger"));
    assert!(expr.requires("Database"));
    assert!(!expr.requires("FileSystem"));

    // Variables always return false for requires
    let var_expr = ContextExpr::variable(TypeVar::fresh());
    assert!(!var_expr.requires("Logger"));
}

#[test]
fn test_context_expr_iter() {
    let expr = ContextExpr::concrete(make_logger_db_req());
    let names: Vec<&str> = expr.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"Logger"));
    assert!(names.contains(&"Database"));
    assert_eq!(names.len(), 2);
}

#[test]
fn test_context_expr_from_requirement() {
    let req = make_logger_req();
    let expr: ContextExpr = req.clone().into();
    assert!(expr.is_concrete());
    assert_eq!(expr.len(), 1);
}

#[test]
fn test_context_expr_from_type_var() {
    let var = TypeVar::fresh();
    let expr: ContextExpr = var.into();
    assert!(expr.is_variable());
}

// ============================================================================
// ContextRef properties
// ============================================================================

#[test]
fn test_context_ref_new() {
    let ctx = ContextRef::new("MyContext".into(), TypeId::of::<()>());
    assert_eq!(ctx.name.as_str(), "MyContext");
    assert!(!ctx.is_async);
    assert!(!ctx.is_negative);
    assert!(ctx.alias.is_none());
}

#[test]
fn test_context_ref_equality() {
    let ctx1 = ContextRef::new("Logger".into(), TypeId::of::<()>());
    let ctx2 = ContextRef::new("Logger".into(), TypeId::of::<()>());
    assert_eq!(ctx1, ctx2);
}

#[test]
fn test_context_ref_inequality_by_name() {
    let ctx1 = ContextRef::new("Logger".into(), TypeId::of::<()>());
    let ctx2 = ContextRef::new("Database".into(), TypeId::of::<()>());
    assert_ne!(ctx1, ctx2);
}

// ============================================================================
// Context unification through the type system
// ============================================================================

#[test]
fn test_unify_function_types_with_same_contexts() {
    let mut unifier = Unifier::new();
    let span = make_span();

    let logger_ctx = ContextExpr::concrete(make_logger_req());

    let f1 = Type::Function {
        params: List::from(vec![Type::Int]),
        return_type: Box::new(Type::Int),
        contexts: Some(logger_ctx.clone()),
        type_params: List::new(),
        properties: None,
    };

    let f2 = Type::Function {
        params: List::from(vec![Type::Int]),
        return_type: Box::new(Type::Int),
        contexts: Some(logger_ctx),
        type_params: List::new(),
        properties: None,
    };

    // Same contexts should unify successfully
    let result = unifier.unify(&f1, &f2, span);
    assert!(result.is_ok(), "Same-context function types should unify");
}

#[test]
fn test_unify_function_types_with_no_contexts() {
    let mut unifier = Unifier::new();
    let span = make_span();

    let f1 = Type::Function {
        params: List::from(vec![Type::Int]),
        return_type: Box::new(Type::Bool),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };

    let f2 = Type::Function {
        params: List::from(vec![Type::Int]),
        return_type: Box::new(Type::Bool),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };

    let result = unifier.unify(&f1, &f2, span);
    assert!(result.is_ok(), "Context-free function types should unify");
}

#[test]
fn test_unify_function_types_with_context_variable() {
    let mut unifier = Unifier::new();
    let span = make_span();

    let ctx_var = TypeVar::fresh();

    // f1 uses a context variable C
    let f1 = Type::Function {
        params: List::from(vec![Type::Int]),
        return_type: Box::new(Type::Int),
        contexts: Some(ContextExpr::variable(ctx_var)),
        type_params: List::new(),
        properties: None,
    };

    // f2 uses concrete Logger context
    let f2 = Type::Function {
        params: List::from(vec![Type::Int]),
        return_type: Box::new(Type::Int),
        contexts: Some(ContextExpr::concrete(make_logger_req())),
        type_params: List::new(),
        properties: None,
    };

    // Context variable should unify with concrete context
    let result = unifier.unify(&f1, &f2, span);
    assert!(
        result.is_ok(),
        "Context variable should unify with concrete context: {:?}",
        result.err()
    );
}

#[test]
fn test_unify_function_types_context_mismatch() {
    let mut unifier = Unifier::new();
    let span = make_span();

    let f1 = Type::Function {
        params: List::from(vec![Type::Int]),
        return_type: Box::new(Type::Int),
        contexts: Some(ContextExpr::concrete(make_logger_req())),
        type_params: List::new(),
        properties: None,
    };

    let f2 = Type::Function {
        params: List::from(vec![Type::Int]),
        return_type: Box::new(Type::Int),
        contexts: Some(ContextExpr::concrete(make_db_req())),
        type_params: List::new(),
        properties: None,
    };

    // Different concrete contexts should fail to unify
    let result = unifier.unify(&f1, &f2, span);
    assert!(
        result.is_err(),
        "Different concrete contexts should not unify"
    );
}

#[test]
fn test_context_requirement_context_names() {
    let req = make_logger_db_req();
    let names = req.context_names();
    let name_strs: Vec<&str> = names.iter().map(|t| t.as_str()).collect();
    assert!(name_strs.contains(&"Logger"));
    assert!(name_strs.contains(&"Database"));
}

#[test]
fn test_context_expr_context_names() {
    let expr = ContextExpr::concrete(make_logger_db_req());
    let names = expr.context_names();
    assert_eq!(names.len(), 2);

    // Variable should have no names
    let var_expr = ContextExpr::variable(TypeVar::fresh());
    let var_names = var_expr.context_names();
    assert_eq!(var_names.len(), 0);
}

// ============================================================================
// ContextExpr display
// ============================================================================

#[test]
fn test_context_expr_display_concrete() {
    let expr = ContextExpr::concrete(make_logger_req());
    let display = format!("{}", expr);
    assert!(
        display.contains("Logger"),
        "Display should contain Logger: got '{}'",
        display
    );
}

#[test]
fn test_context_expr_display_variable() {
    let var = TypeVar::fresh();
    let expr = ContextExpr::variable(var);
    let display = format!("{}", expr);
    // TypeVar display format
    assert!(!display.is_empty());
}

// ============================================================================
// ContextExpr serialization roundtrip
// ============================================================================

#[test]
fn test_context_expr_serialize_concrete() {
    // ContextExpr serializes with a struct envelope {kind, value}
    // but its Deserialize impl delegates directly to ContextRequirement (for simplicity).
    // So we test serialization and requirement-level roundtrip separately.
    let expr = ContextExpr::concrete(make_logger_db_req());
    let json = serde_json::to_string(&expr).expect("serialize");
    // Verify the JSON contains the context info
    assert!(json.contains("Logger"));
    assert!(json.contains("Database"));

    // Test ContextRequirement roundtrip directly (which is what Deserialize supports)
    let req = make_logger_db_req();
    let req_json = serde_json::to_string(&req).expect("serialize req");
    let deserialized_req: ContextRequirement = serde_json::from_str(&req_json).expect("deserialize req");
    assert_eq!(deserialized_req.len(), 2);
    assert!(deserialized_req.requires("Logger"));
    assert!(deserialized_req.requires("Database"));
}

#[test]
fn test_context_requirement_serialize_empty() {
    let req = ContextRequirement::empty();
    let json = serde_json::to_string(&req).expect("serialize");
    let deserialized: ContextRequirement = serde_json::from_str(&json).expect("deserialize");
    assert!(deserialized.is_empty());
}

#[test]
fn test_context_requirement_serialize_with_contexts() {
    let req = make_logger_db_req();
    let json = serde_json::to_string(&req).expect("serialize");
    let deserialized: ContextRequirement = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deserialized.len(), 2);
    assert!(deserialized.requires("Logger"));
    assert!(deserialized.requires("Database"));
}
