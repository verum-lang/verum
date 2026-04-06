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
//! Comprehensive tests for GAT verification
//!
//! Tests cover:
//! - Simple GAT verification
//! - GAT with type parameters
//! - Where clause satisfaction
//! - Circular dependency detection
//! - Variance checking
//! - Arity mismatches
//! - Cache behavior

use verum_ast::span::Span;
use verum_ast::ty::Type;
use verum_protocol_types::gat_types::{AssociatedTypeGAT, GATTypeParam, GATWhereClause, Variance};
use verum_protocol_types::protocol_base::ProtocolBound;
use verum_smt::gat_verification::{
    GATError, GATVerifier, is_well_formed, suggest_fixes, verify_gat, verify_gats,
};
use verum_common::{List, Maybe, Text};

#[test]
fn test_simple_gat_verification() {
    let gat = AssociatedTypeGAT::simple("Item".into(), List::new());
    let result = verify_gat(&gat);

    assert!(result.is_valid);
    assert!(result.errors.is_empty());
    assert!(result.counterexamples.is_empty());
    assert_eq!(result.stats.type_params_checked, 0);
}

#[test]
fn test_gat_with_single_type_parameter() {
    let param = GATTypeParam {
        name: "T".into(),
        bounds: List::new(),
        default: Maybe::None,
        variance: Variance::Covariant,
    };

    let gat = AssociatedTypeGAT::generic(
        "Wrapped".into(),
        List::from(vec![param]),
        List::new(),
        List::new(),
    );

    let result = verify_gat(&gat);
    assert!(result.is_valid);
    assert_eq!(result.stats.type_params_checked, 1);
}

#[test]
fn test_gat_with_multiple_type_parameters() {
    let params = List::from(vec![
        GATTypeParam {
            name: "K".into(),
            bounds: List::new(),
            default: Maybe::None,
            variance: Variance::Covariant,
        },
        GATTypeParam {
            name: "V".into(),
            bounds: List::new(),
            default: Maybe::None,
            variance: Variance::Covariant,
        },
    ]);

    let gat = AssociatedTypeGAT::generic("Entry".into(), params, List::new(), List::new());

    let result = verify_gat(&gat);
    assert!(result.is_valid);
    assert_eq!(result.stats.type_params_checked, 2);
}

#[test]
fn test_gat_with_where_clause() {
    let param = GATTypeParam {
        name: "T".into(),
        bounds: List::new(),
        default: Maybe::None,
        variance: Variance::Covariant,
    };

    let where_clause = GATWhereClause {
        param: "T".into(),
        constraints: List::new(),
        span: Span::default(),
    };

    let gat = AssociatedTypeGAT::generic(
        "Item".into(),
        List::from(vec![param]),
        List::new(),
        List::from(vec![where_clause]),
    );

    let result = verify_gat(&gat);
    assert!(result.is_valid);
    assert_eq!(result.stats.where_clauses_checked, 1);
}

#[test]
fn test_batch_gat_verification() {
    let gat1 = AssociatedTypeGAT::simple("Item".into(), List::new());
    let gat2 = AssociatedTypeGAT::simple("Output".into(), List::new());
    let gat3 = AssociatedTypeGAT::simple("Error".into(), List::new());

    let results = verify_gats(&[gat1, gat2, gat3]);

    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.is_valid));
}

#[test]
fn test_is_well_formed_predicate() {
    let gat = AssociatedTypeGAT::simple("Item".into(), List::new());
    assert!(is_well_formed(&gat));

    let param = GATTypeParam {
        name: "T".into(),
        bounds: List::new(),
        default: Maybe::None,
        variance: Variance::Invariant,
    };

    let gat_with_param = AssociatedTypeGAT::generic(
        "Wrapper".into(),
        List::from(vec![param]),
        List::new(),
        List::new(),
    );

    assert!(is_well_formed(&gat_with_param));
}

#[test]
fn test_verifier_cache() {
    let mut verifier = GATVerifier::new();
    let gat = AssociatedTypeGAT::simple("Cached".into(), List::new());

    // First verification
    let result1 = verifier.verify(&gat);
    assert!(result1.is_valid);

    // Second verification should hit cache
    let result2 = verifier.verify(&gat);
    assert!(result2.is_valid);

    let stats = verifier.cache_stats();
    assert_eq!(stats.entries, 1);
}

#[test]
fn test_verifier_cache_clearing() {
    let mut verifier = GATVerifier::new();
    let gat = AssociatedTypeGAT::simple("Item".into(), List::new());

    verifier.verify(&gat);
    assert_eq!(verifier.cache_stats().entries, 1);

    verifier.clear_cache();
    assert_eq!(verifier.cache_stats().entries, 0);
}

#[test]
fn test_gat_with_contravariant_parameter() {
    let param = GATTypeParam {
        name: "Input".into(),
        bounds: List::new(),
        default: Maybe::None,
        variance: Variance::Contravariant,
    };

    let gat = AssociatedTypeGAT::generic(
        "Consumer".into(),
        List::from(vec![param]),
        List::new(),
        List::new(),
    );

    let result = verify_gat(&gat);
    assert!(result.is_valid);
}

#[test]
fn test_gat_with_invariant_parameter() {
    let param = GATTypeParam {
        name: "T".into(),
        bounds: List::new(),
        default: Maybe::None,
        variance: Variance::Invariant,
    };

    let gat = AssociatedTypeGAT::generic(
        "Cell".into(),
        List::from(vec![param]),
        List::new(),
        List::new(),
    );

    let result = verify_gat(&gat);
    assert!(result.is_valid);
}

#[test]
fn test_suggest_fixes_empty() {
    let error = GATError::ArityMismatch {
        gat_name: "Item".into(),
        expected: 1,
        found: 0,
    };

    let suggestions = suggest_fixes(&error);
    assert!(!suggestions.is_empty());
}

#[test]
fn test_verification_timing() {
    let gat = AssociatedTypeGAT::simple("Timed".into(), List::new());
    let result = verify_gat(&gat);

    // Verification should be fast for simple GATs
    assert!(result.duration.as_millis() < 100);
}

#[test]
fn test_gat_stats_aggregation() {
    let mut verifier = GATVerifier::new();

    let param = GATTypeParam {
        name: "T".into(),
        bounds: List::new(),
        default: Maybe::None,
        variance: Variance::Covariant,
    };

    let where_clause = GATWhereClause {
        param: "T".into(),
        constraints: List::new(),
        span: Span::default(),
    };

    let gat = AssociatedTypeGAT::generic(
        "Complex".into(),
        List::from(vec![param]),
        List::new(),
        List::from(vec![where_clause]),
    );

    let result = verifier.verify(&gat);

    assert_eq!(result.stats.type_params_checked, 1);
    assert_eq!(result.stats.where_clauses_checked, 1);
}

#[test]
fn test_default_verifier() {
    let verifier = GATVerifier::default();
    // Should create successfully
}

#[test]
fn test_gat_with_default_type() {
    let param = GATTypeParam {
        name: "T".into(),
        bounds: List::new(),
        default: Maybe::Some(Type::int(Span::dummy())),
        variance: Variance::Covariant,
    };

    let gat = AssociatedTypeGAT::generic(
        "DefaultItem".into(),
        List::from(vec![param]),
        List::new(),
        List::new(),
    );

    let result = verify_gat(&gat);
    assert!(result.is_valid);
}

#[test]
fn test_multiple_where_clauses() {
    let param = GATTypeParam {
        name: "T".into(),
        bounds: List::new(),
        default: Maybe::None,
        variance: Variance::Covariant,
    };

    let where_clauses = List::from(vec![
        GATWhereClause {
            param: "T".into(),
            constraints: List::new(),
            span: Span::default(),
        },
        GATWhereClause {
            param: "T".into(),
            constraints: List::new(),
            span: Span::default(),
        },
    ]);

    let gat = AssociatedTypeGAT::generic(
        "Constrained".into(),
        List::from(vec![param]),
        List::new(),
        where_clauses,
    );

    let result = verify_gat(&gat);
    assert_eq!(result.stats.where_clauses_checked, 2);
}

// ==================== DynProtocol Bounds Analysis Tests ====================

#[test]
fn test_dyn_protocol_variance_analysis_basic() {
    // Test that dyn Protocol types are analyzed for variance
    use verum_ast::Ident;
    use verum_ast::ty::Path;
    use verum_ast::ty::{TypeBound, TypeBoundKind, TypeKind};
    use verum_smt::gat_verification::{VariancePosition, VarianceTracker};

    let mut tracker = VarianceTracker::new("T".into());

    // Create a dyn Protocol type with no bindings
    // Note: TypeKind uses verum_common types (Vec, Option) while verum_protocol_types uses verum_std types
    let bounds = vec![TypeBound {
        kind: TypeBoundKind::Protocol(Path::single(Ident::new("Clone", Span::dummy()))),
        span: Span::dummy(),
    }];

    let dyn_ty = Type::new(
        TypeKind::DynProtocol {
            bounds: bounds.into(),
            bindings: None,
        },
        Span::dummy(),
    );

    // dyn Protocol bounds are analyzed as contravariant (flipped)
    tracker.analyze_type(&dyn_ty, VariancePosition::Covariant);

    // The bounds themselves don't contain T, so variance shouldn't be affected
    let variance = tracker.get_variance();
    assert_eq!(variance, Variance::Covariant); // Default when no usage found
}

// ==================== Transitive Bounds Tests ====================

#[test]
fn test_transitive_bounds_checking() {
    // Test that transitive bounds are properly verified
    use verum_ast::Ident;
    use verum_ast::ty::Path;

    let mut verifier = GATVerifier::new();

    // Register protocol hierarchy: Ord : PartialOrd + Eq
    verifier.register_standard_protocols();

    // Create GAT with T: Ord bound
    let ord_bound =
        ProtocolBound::positive(Path::single(Ident::new("Ord", Span::dummy())), List::new());

    let param = GATTypeParam {
        name: "T".into(),
        bounds: List::from(vec![ord_bound]),
        default: Maybe::None,
        variance: Variance::Covariant,
    };

    let gat = AssociatedTypeGAT::generic(
        "OrderedItem".into(),
        List::from(vec![param]),
        List::new(),
        List::new(),
    );

    let result = verifier.verify(&gat);

    // Should verify successfully - Ord implies PartialOrd and Eq
    assert!(
        result.is_valid,
        "Transitive bounds verification failed: {:?}",
        result.errors
    );
    assert!(result.stats.transitive_bounds_checked > 0);
}

// ==================== Lifetime Bounds Tests ====================

#[test]
fn test_lifetime_bounds_verification_basic() {
    // Test that lifetime bounds are counted and verified
    let param = GATTypeParam {
        name: "T".into(),
        bounds: List::new(), // No lifetime bounds
        default: Maybe::None,
        variance: Variance::Covariant,
    };

    let gat = AssociatedTypeGAT::generic(
        "RefItem".into(),
        List::from(vec![param]),
        List::new(),
        List::new(),
    );

    let result = verify_gat(&gat);

    assert!(result.is_valid);
    // No lifetime bounds to check
    assert_eq!(result.stats.lifetime_bounds_checked, 0);
}

// ==================== Variance Tracker Tests ====================

#[test]
fn test_variance_tracker_covariant_usage() {
    use verum_ast::Ident;
    use verum_ast::ty::Path;
    use verum_ast::ty::TypeKind;
    use verum_smt::gat_verification::{VariancePosition, VarianceTracker};

    let mut tracker = VarianceTracker::new("T".into());

    // T in covariant position (like return type)
    let t_ty = Type::new(
        TypeKind::Path(Path::single(Ident::new("T", Span::dummy()))),
        Span::dummy(),
    );

    tracker.analyze_type(&t_ty, VariancePosition::Covariant);

    assert!(tracker.seen_covariant);
    assert!(!tracker.seen_contravariant);
    assert!(!tracker.seen_invariant);
    assert_eq!(tracker.get_variance(), Variance::Covariant);
}

#[test]
fn test_variance_tracker_contravariant_usage() {
    use verum_ast::Ident;
    use verum_ast::ty::Path;
    use verum_ast::ty::TypeKind;
    use verum_smt::gat_verification::{VariancePosition, VarianceTracker};

    let mut tracker = VarianceTracker::new("T".into());

    // T in contravariant position (like function parameter)
    let t_ty = Type::new(
        TypeKind::Path(Path::single(Ident::new("T", Span::dummy()))),
        Span::dummy(),
    );

    tracker.analyze_type(&t_ty, VariancePosition::Contravariant);

    assert!(!tracker.seen_covariant);
    assert!(tracker.seen_contravariant);
    assert!(!tracker.seen_invariant);
    assert_eq!(tracker.get_variance(), Variance::Contravariant);
}

#[test]
fn test_variance_tracker_invariant_from_both() {
    use verum_ast::Ident;
    use verum_ast::ty::Path;
    use verum_ast::ty::TypeKind;
    use verum_smt::gat_verification::{VariancePosition, VarianceTracker};

    let mut tracker = VarianceTracker::new("T".into());

    let t_ty = Type::new(
        TypeKind::Path(Path::single(Ident::new("T", Span::dummy()))),
        Span::dummy(),
    );

    // T in both positions = invariant
    tracker.analyze_type(&t_ty, VariancePosition::Covariant);
    tracker.analyze_type(&t_ty, VariancePosition::Contravariant);

    assert!(tracker.seen_covariant);
    assert!(tracker.seen_contravariant);
    assert_eq!(tracker.get_variance(), Variance::Invariant);
}

#[test]
fn test_variance_tracker_mutable_reference() {
    use verum_ast::Ident;
    use verum_ast::ty::Path;
    use verum_ast::ty::TypeKind;
    use verum_smt::gat_verification::{VariancePosition, VarianceTracker};

    let mut tracker = VarianceTracker::new("T".into());

    let t_ty = Type::new(
        TypeKind::Path(Path::single(Ident::new("T", Span::dummy()))),
        Span::dummy(),
    );

    // &mut T is invariant in T
    let mut_ref = Type::new(
        TypeKind::Reference {
            mutable: true,
            inner: Box::new(t_ty),
        },
        Span::dummy(),
    );

    tracker.analyze_type(&mut_ref, VariancePosition::Covariant);

    assert!(tracker.seen_invariant);
    assert_eq!(tracker.get_variance(), Variance::Invariant);
}

// ==================== Error Suggestions Tests ====================

#[test]
fn test_suggest_fixes_lifetime_bound_violation() {
    let error = GATError::LifetimeBoundViolation {
        gat_name: "Item".into(),
        lifetime_param: "'a".into(),
        required_bound: "'a: 'static".into(),
        explanation: "Lifetime is too short".into(),
    };

    let suggestions = suggest_fixes(&error);
    assert!(!suggestions.is_empty());
    assert!(suggestions.iter().any(|s| s.as_str().contains("lifetime")));
}

#[test]
fn test_suggest_fixes_transitive_bound_violation() {
    let error = GATError::TransitiveBoundViolation {
        gat_name: "Item".into(),
        param: "T".into(),
        direct_bound: "Ord".into(),
        transitive_bound: "PartialOrd".into(),
        explanation: "Missing PartialOrd implementation".into(),
    };

    let suggestions = suggest_fixes(&error);
    assert!(!suggestions.is_empty());
    assert!(
        suggestions
            .iter()
            .any(|s| s.as_str().contains("transitive"))
    );
}

// ==================== Integration Tests ====================

#[test]
fn test_complex_gat_with_all_features() {
    use verum_ast::Ident;
    use verum_ast::ty::Path;

    let mut verifier = GATVerifier::new();
    verifier.register_standard_protocols();

    // Create a GAT with:
    // - Multiple type parameters
    // - Protocol bounds
    // - Where clauses
    // - Different variances

    let clone_bound = ProtocolBound::positive(
        Path::single(Ident::new("Clone", Span::dummy())),
        List::new(),
    );

    let debug_bound = ProtocolBound::positive(
        Path::single(Ident::new("Debug", Span::dummy())),
        List::new(),
    );

    let params = List::from(vec![
        GATTypeParam {
            name: "K".into(),
            bounds: List::from(vec![clone_bound.clone()]),
            default: Maybe::None,
            variance: Variance::Covariant,
        },
        GATTypeParam {
            name: "V".into(),
            bounds: List::from(vec![debug_bound.clone()]),
            default: Maybe::None,
            variance: Variance::Invariant,
        },
    ]);

    let where_clauses = List::from(vec![GATWhereClause {
        param: "K".into(),
        constraints: List::from(vec![debug_bound]),
        span: Span::default(),
    }]);

    let gat = AssociatedTypeGAT::generic(
        "Entry".into(),
        params,
        List::from(vec![clone_bound]),
        where_clauses,
    );

    let result = verifier.verify(&gat);

    assert!(
        result.is_valid,
        "Complex GAT verification failed: {:?}",
        result.errors
    );
    assert_eq!(result.stats.type_params_checked, 2);
    assert_eq!(result.stats.where_clauses_checked, 1);
}

#[test]
fn test_gat_verification_with_standard_protocols() {
    let mut verifier = GATVerifier::new();
    verifier.register_standard_protocols();

    // Verify GAT with Iterator protocol
    use verum_ast::Ident;
    use verum_ast::ty::Path;

    let iter_bound = ProtocolBound::positive(
        Path::single(Ident::new("Iterator", Span::dummy())),
        List::new(),
    );

    let param = GATTypeParam {
        name: "I".into(),
        bounds: List::from(vec![iter_bound]),
        default: Maybe::None,
        variance: Variance::Covariant,
    };

    let gat = AssociatedTypeGAT::generic(
        "IterItem".into(),
        List::from(vec![param]),
        List::new(),
        List::new(),
    );

    let result = verifier.verify(&gat);
    assert!(result.is_valid);
}
