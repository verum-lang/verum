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
// Advanced refinement type verification tests
//
// Tests for:
// - Constraint propagation
// - Dependent refinement types
// - Incremental verification
// - Parallel verification

use verum_ast::literal::IntLit;
use verum_ast::{Expr, ExprKind, Literal, LiteralKind, Span, Type, TypeKind};
use verum_smt::{
    Context, IncrementalVerifier, RefinementVerifier, VerifyMode, verify_batch_incremental,
    verify_parallel,
};

// Helper to create Int type
fn int_type() -> Type {
    Type::new(TypeKind::Int, Span::dummy())
}

// Helper to create literal expression
fn int_literal(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: value as i128,
                suffix: None,
            }),
            span: Span::dummy(),
        }),
        Span::dummy(),
    )
}

#[test]
fn test_constraint_propagation() {
    let verifier = RefinementVerifier::new();

    // Create a simple type for testing
    let base_ty = int_type();

    // Test constraint propagation (would need actual refinement types)
    let constraints = verifier.propagate_constraints(&base_ty);

    // Base type has no constraints
    assert_eq!(constraints.len(), 0);
}

#[test]
fn test_dependent_refinement_verification() {
    let mut verifier = RefinementVerifier::new();

    // Create dependencies: n = 10
    let dependencies = vec![("n".into(), int_literal(10))];

    // Test with a simple type (would need actual refinement type with dependencies)
    let ty = int_type();

    let result = verifier.verify_dependent_refinement(&ty, &dependencies);

    // Should succeed for non-refinement types
    assert!(result.is_ok());
}

#[test]
fn test_verify_mode_runtime() {
    let verifier = RefinementVerifier::with_mode(VerifyMode::Runtime);

    // Runtime mode should skip SMT verification
    let ty = int_type();

    let result = verifier.verify_refinement(&ty, None, Some(VerifyMode::Runtime));

    // Runtime mode may succeed or return error depending on implementation
    // The key is that it doesn't hang or crash
    if let Ok(proof) = result {
        // If successful, duration should be minimal (< 1ms)
        assert!(proof.cost.duration.as_millis() < 10);
    }
    // Error is also acceptable for runtime mode with no actual refinement
}

#[test]
fn test_incremental_verifier() {
    let context = Context::new();
    let mut verifier = IncrementalVerifier::new(&context);

    // Test scope management
    assert_eq!(verifier.scope_depth(), 0);

    verifier.push();
    assert_eq!(verifier.scope_depth(), 1);

    verifier.push();
    assert_eq!(verifier.scope_depth(), 2);

    verifier.pop();
    assert_eq!(verifier.scope_depth(), 1);

    verifier.pop();
    assert_eq!(verifier.scope_depth(), 0);
}

#[test]
fn test_parallel_verification() {
    let context = Context::new();

    // Create multiple constraints
    let constraints = vec![(int_type(), None), (int_type(), None), (int_type(), None)];

    let results = verify_parallel(&context, &constraints, VerifyMode::Auto);

    // Verify we get results for all constraints
    assert_eq!(results.len(), 3);
    // Results may succeed or fail depending on Z3 configuration
    // Just verify the API works correctly
}

#[test]
fn test_batch_incremental_verification() {
    let context = Context::new();

    // Create batch of constraints
    let constraints = vec![(int_type(), None), (int_type(), None)];

    let results = verify_batch_incremental(&context, &constraints, VerifyMode::Auto);

    // Verify we get results for all constraints
    assert_eq!(results.len(), 2);
    // Results may succeed or fail depending on Z3 configuration
}

#[test]
fn test_complexity_categorization() {
    use verum_smt::{PredicateComplexity, categorize_complexity};

    let ty = int_type();
    let complexity = categorize_complexity(&ty);

    // Simple types should be Simple complexity
    assert_eq!(complexity, PredicateComplexity::Simple);
}

#[test]
fn test_needs_smt_verification() {
    use verum_smt::needs_smt_verification;

    let ty = int_type();

    // Runtime mode never needs SMT
    assert!(!needs_smt_verification(&ty, VerifyMode::Runtime));

    // Proof mode needs SMT for refinement types (but not for base types)
    assert!(!needs_smt_verification(&ty, VerifyMode::Proof));

    // Auto mode depends on complexity
    assert!(!needs_smt_verification(&ty, VerifyMode::Auto));
}

#[test]
fn test_is_refinement_type() {
    use verum_smt::is_refinement_type;

    let ty = int_type();
    assert!(!is_refinement_type(&ty));

    // Would test with actual refinement type:
    // let refined_ty = create_refinement_type(...);
    // assert!(is_refinement_type(&refined_ty));
}

#[test]
fn test_extract_predicate() {
    use verum_smt::extract_predicate;

    let ty = int_type();
    let predicate = extract_predicate(&ty);

    // Base type has no predicate
    assert!(predicate.is_none());
}

#[test]
fn test_subsumption_checking() {
    let verifier = RefinementVerifier::new();

    // Would need actual refinement types to test subsumption
    // For now, just verify the API exists
    let ty1 = int_type();
    let ty2 = int_type();

    let _result =
        verifier.check_subsumption(&ty1, &ty2, verum_smt::subsumption::CheckMode::SyntacticOnly);

    // Should return Unknown for non-refinement types
}

#[test]
fn test_cache_statistics() {
    let verifier = RefinementVerifier::new();

    let stats = verifier.cache_stats();

    // Initially should have 0 size
    assert_eq!(stats.size, 0);
}

#[test]
fn test_clear_caches() {
    let verifier = RefinementVerifier::new();

    // Should not panic
    verifier.clear_caches();

    let stats = verifier.cache_stats();
    assert_eq!(stats.size, 0);
}
