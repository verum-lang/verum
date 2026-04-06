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
// Unit tests for context.rs
//
// Migrated from src/context.rs per project test organization guidelines.

use verum_common::List;
use verum_types::context::*;
use verum_types::ty::{Type, TypeVar};

#[test]
fn test_env_lookup() {
    let mut env = TypeEnv::new();
    env.insert_mono("x".to_string(), Type::int());
    env.insert_mono("y".to_string(), Type::bool());

    assert!(matches!(env.lookup("x").map(|s| &s.ty), Some(Type::Int)));
    assert!(matches!(env.lookup("y").map(|s| &s.ty), Some(Type::Bool)));
    assert!(env.lookup("z").is_none());
}

#[test]
fn test_env_scope() {
    let mut env = TypeEnv::new();
    env.insert_mono("x".to_string(), Type::int());

    let mut child = env.child();
    child.insert_mono("y".to_string(), Type::bool());

    // Child can see parent bindings
    assert!(child.lookup("x").is_some());
    assert!(child.lookup("y").is_some());

    // Parent cannot see child bindings
    assert!(env.lookup("x").is_some());
    assert!(env.lookup("y").is_none());
}

#[test]
fn test_generalize() {
    let mut env = TypeEnv::new();

    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    // Environment has v1
    env.insert_mono("x".to_string(), Type::Var(v1));

    // Type has v1 and v2
    let ty = Type::function(vec![Type::Var(v1)].into(), Type::Var(v2));

    let scheme = env.generalize(ty);

    // Should only quantify v2 (v1 is in environment)
    assert_eq!(scheme.vars.len(), 1);
    assert!(scheme.vars.contains(&v2));
}

#[test]
fn test_context_protocols() {
    let ctx = TypeContext::new();

    assert!(ctx.implements_protocol("Int", "Eq"));
    assert!(ctx.implements_protocol("Int", "Add"));
    // Verum uses "Text" instead of "String" (semantic type naming)
    assert!(ctx.implements_protocol("Text", "Eq"));
    assert!(!ctx.implements_protocol("Text", "Add"));
}

// =============================================================================
// Tests for Protocol Bound Tracking
// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4 - GAT where clause constraints
// =============================================================================

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_types::protocol::ProtocolBound;

#[test]
fn test_add_protocol_bound() {
    // Test that protocol bounds can be added to type variables
    // Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4
    let mut ctx = TypeContext::new();
    let var = TypeVar::fresh();

    // Add a protocol bound: T: Clone
    let clone_bound = ProtocolBound {
        protocol: Path::single(Ident::new("Clone", Span::default())),
        args: List::new(),
        is_negative: false,
    };

    ctx.add_protocol_bound(var, clone_bound);

    // Verify the bound was added
    if let Option::Some(bounds) = ctx.get_protocol_bounds(&var) {
        assert_eq!(bounds.len(), 1);
    } else {
        panic!("Expected protocol bounds to be found");
    }
}

#[test]
fn test_multiple_protocol_bounds() {
    // Test adding multiple protocol bounds to a single type variable
    // Example: fn sort<T: Ord + Clone>(list: List<T>) -> List<T>
    let mut ctx = TypeContext::new();
    let var = TypeVar::fresh();

    // Add T: Ord
    let ord_bound = ProtocolBound {
        protocol: Path::single(Ident::new("Ord", Span::default())),
        args: List::new(),
        is_negative: false,
    };
    ctx.add_protocol_bound(var, ord_bound);

    // Add T: Clone
    let clone_bound = ProtocolBound {
        protocol: Path::single(Ident::new("Clone", Span::default())),
        args: List::new(),
        is_negative: false,
    };
    ctx.add_protocol_bound(var, clone_bound);

    // Verify both bounds exist
    if let Option::Some(bounds) = ctx.get_protocol_bounds(&var) {
        assert_eq!(bounds.len(), 2);
    } else {
        panic!("Expected protocol bounds to be found");
    }
}

#[test]
fn test_has_protocol_bound() {
    // Test checking if a type variable has a specific bound
    let mut ctx = TypeContext::new();
    let var = TypeVar::fresh();

    let clone_bound = ProtocolBound {
        protocol: Path::single(Ident::new("Clone", Span::default())),
        args: List::new(),
        is_negative: false,
    };
    ctx.add_protocol_bound(var, clone_bound);

    // Check for existing bound
    assert!(ctx.has_protocol_bound(&var, &"Clone".into()));

    // Check for non-existing bound
    assert!(!ctx.has_protocol_bound(&var, &"Send".into()));
}

#[test]
fn test_clear_protocol_bounds() {
    // Test removing protocol bounds for a type variable
    let mut ctx = TypeContext::new();
    let var = TypeVar::fresh();

    let clone_bound = ProtocolBound {
        protocol: Path::single(Ident::new("Clone", Span::default())),
        args: List::new(),
        is_negative: false,
    };
    ctx.add_protocol_bound(var, clone_bound);

    // Verify bound exists
    assert!(ctx.has_protocol_bound(&var, &"Clone".into()));

    // Clear bounds
    ctx.clear_protocol_bounds(&var);

    // Verify bound is gone
    assert!(!ctx.has_protocol_bound(&var, &"Clone".into()));
}

#[test]
fn test_negative_protocol_bound() {
    // Test negative protocol bounds for specialization
    // Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — .4 - Negative Reasoning
    let mut ctx = TypeContext::new();
    let var = TypeVar::fresh();

    // Add T: !Sync (type must NOT implement Sync)
    let not_sync_bound = ProtocolBound {
        protocol: Path::single(Ident::new("Sync", Span::default())),
        args: List::new(),
        is_negative: true,
    };
    ctx.add_protocol_bound(var, not_sync_bound);

    // Verify the bound exists and is retrievable
    if let Option::Some(bounds) = ctx.get_protocol_bounds(&var) {
        assert_eq!(bounds.len(), 1);
        assert!(bounds.first().unwrap().is_negative_bound());
    } else {
        panic!("Expected protocol bounds to be found");
    }
}

#[test]
fn test_protocol_bounds_independent_vars() {
    // Test that bounds are tracked independently per type variable
    let mut ctx = TypeContext::new();
    let var1 = TypeVar::fresh();
    let var2 = TypeVar::fresh();

    // Add T1: Clone
    let clone_bound = ProtocolBound {
        protocol: Path::single(Ident::new("Clone", Span::default())),
        args: List::new(),
        is_negative: false,
    };
    ctx.add_protocol_bound(var1, clone_bound);

    // Add T2: Send
    let send_bound = ProtocolBound {
        protocol: Path::single(Ident::new("Send", Span::default())),
        args: List::new(),
        is_negative: false,
    };
    ctx.add_protocol_bound(var2, send_bound);

    // Verify bounds are tracked independently
    assert!(ctx.has_protocol_bound(&var1, &"Clone".into()));
    assert!(!ctx.has_protocol_bound(&var1, &"Send".into()));

    assert!(ctx.has_protocol_bound(&var2, &"Send".into()));
    assert!(!ctx.has_protocol_bound(&var2, &"Clone".into()));
}
