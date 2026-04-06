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
// Tests for context module
// Migrated from src/context.rs per CLAUDE.md standards

use verum_common::{List, Maybe, Text};
use verum_verification::context::*;
use verum_verification::{VerificationLevel, VerificationMode};

#[test]
fn test_scope_creation() {
    let scope = VerificationScope::root();
    assert_eq!(scope.id, ScopeId::new(0));
    assert_eq!(scope.parent, Maybe::None);
    assert_eq!(scope.level(), VerificationLevel::Runtime);
}

#[test]
fn test_context_push_pop() {
    let mut ctx = VerificationContext::new();
    let root_id = ctx.current_scope();

    // Push a new scope
    let child_id = ctx.push_scope(VerificationMode::static_mode(), Text::from("child"));
    assert_eq!(ctx.current_scope(), child_id);
    assert_eq!(ctx.current_level(), VerificationLevel::Static);

    // Pop back to root
    ctx.pop_scope().unwrap();
    assert_eq!(ctx.current_scope(), root_id);
    assert_eq!(ctx.current_level(), VerificationLevel::Runtime);
}

#[test]
fn test_valid_transitions() {
    let ctx = VerificationContext::new();

    // Same level
    assert!(ctx.is_valid_transition(VerificationLevel::Runtime, VerificationLevel::Runtime));

    // More restrictive
    assert!(ctx.is_valid_transition(VerificationLevel::Runtime, VerificationLevel::Static));
    assert!(ctx.is_valid_transition(VerificationLevel::Runtime, VerificationLevel::Proof));

    // Less restrictive (requires obligations)
    assert!(!ctx.is_valid_transition(VerificationLevel::Static, VerificationLevel::Runtime));
}

#[test]
fn test_boundary_direction() {
    let boundary = VerificationBoundary {
        id: BoundaryId::new(0),
        from_level: VerificationLevel::Runtime,
        to_level: VerificationLevel::Proof,
        kind: BoundaryKind::FunctionCall,
        obligations: List::new(),
    };

    assert_eq!(boundary.direction(), BoundaryDirection::MoreRestrictive);
    assert!(boundary.requires_obligations());
}
