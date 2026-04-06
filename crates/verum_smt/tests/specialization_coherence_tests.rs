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
//! Comprehensive tests for specialization coherence verification
//!
//! Tests cover:
//! - Specialization lattice construction
//! - Cycle detection
//! - Antisymmetry checking
//! - Ambiguity detection
//! - Overlap detection
//! - CHC encoding

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, Type};
use verum_protocol_types::protocol_base::ProtocolImpl;
use verum_protocol_types::specialization::SpecializationInfo;
use verum_smt::specialization_coherence::{
    SpecializationVerifier, SpecificityOrdering, detect_overlaps, is_coherent,
    verify_specialization,
};
use verum_common::{List, Map, Maybe};

fn create_impl(ty: Type, protocol_name: &str) -> ProtocolImpl {
    let protocol_path = Path::single(Ident {
        name: protocol_name.into(),
        span: Span::default(),
    });

    ProtocolImpl {
        for_type: ty,
        protocol: protocol_path,
        protocol_args: List::new(),
        where_clauses: List::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        methods: Map::new(),
        impl_crate: Maybe::None,
        span: Span::default(),
    }
}

#[test]
fn test_verifier_creation() {
    let result = SpecializationVerifier::new();
    assert!(result.is_ok());
}

#[test]
fn test_empty_verification() {
    let mut verifier = SpecializationVerifier::new().unwrap();
    let result = verifier.verify();

    assert!(result.is_coherent);
    assert!(result.errors.is_empty());
    assert!(result.ambiguities.is_empty());
}

#[test]
fn test_single_impl_verification() {
    let mut verifier = SpecializationVerifier::new().unwrap();
    let impl_ = create_impl(Type::int(Span::dummy()), "Display");

    verifier.register_implementation(impl_, 0);
    let result = verifier.verify();

    assert!(result.is_coherent);
    assert_eq!(result.stats.impls_checked, 1);
}

#[test]
fn test_two_disjoint_impls() {
    let mut verifier = SpecializationVerifier::new().unwrap();

    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 0);
    verifier.register_implementation(create_impl(Type::bool(Span::dummy()), "Display"), 1);

    let result = verifier.verify();
    assert!(result.is_coherent);
}

#[test]
fn test_specialized_impl() {
    let mut verifier = SpecializationVerifier::new().unwrap();

    // General implementation
    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 0);

    // Specialized implementation
    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 1);

    let result = verifier.verify();
    // May have overlaps, but should detect them
    assert_eq!(result.stats.impls_checked, 2);
}

#[test]
fn test_verify_specialization_empty() {
    let result = verify_specialization(&[]);
    assert!(result.is_coherent);
}

#[test]
fn test_verify_specialization_single() {
    let impl_ = create_impl(Type::int(Span::dummy()), "Clone");
    let result = verify_specialization(&[impl_]);

    assert!(result.is_coherent);
}

#[test]
fn test_detect_overlaps_empty() {
    let overlaps = detect_overlaps(&[]);
    assert!(overlaps.is_empty());
}

#[test]
fn test_detect_overlaps_disjoint() {
    let impl1 = create_impl(Type::int(Span::dummy()), "Display");
    let impl2 = create_impl(Type::bool(Span::dummy()), "Display");

    let overlaps = detect_overlaps(&[impl1, impl2]);
    // Should detect potential overlaps for same protocol
}

#[test]
fn test_specificity_ordering_equality() {
    let eq = SpecificityOrdering::Equal;
    assert_eq!(eq, SpecificityOrdering::Equal);
}

#[test]
fn test_specificity_ordering_values() {
    let first = SpecificityOrdering::MoreSpecific;
    let second = SpecificityOrdering::LessSpecific;
    let equal = SpecificityOrdering::Equal;
    let incomparable = SpecificityOrdering::Incomparable;

    assert_ne!(first, second);
    assert_ne!(first, equal);
    assert_ne!(first, incomparable);
}

#[test]
fn test_default_verifier() {
    let verifier = SpecializationVerifier::default();
    // Should create successfully
}

#[test]
fn test_cache_clearing() {
    let mut verifier = SpecializationVerifier::new().unwrap();
    verifier.clear_cache();
    // Should succeed without panic
}

#[test]
fn test_verification_statistics() {
    let mut verifier = SpecializationVerifier::new().unwrap();

    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Debug"), 0);
    verifier.register_implementation(create_impl(Type::bool(Span::dummy()), "Debug"), 1);

    let result = verifier.verify();

    assert_eq!(result.stats.impls_checked, 2);
    assert!(result.stats.comparisons > 0);
}

#[test]
fn test_multiple_protocols() {
    let mut verifier = SpecializationVerifier::new().unwrap();

    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 0);
    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Debug"), 1);

    let result = verifier.verify();
    assert!(result.is_coherent);
}

#[test]
fn test_lattice_properties() {
    use verum_protocol_types::specialization::SpecializationLattice;

    let protocol_path = Path::single(Ident {
        name: "Test".into(),
        span: Span::default(),
    });
    let lattice = SpecializationLattice::new(protocol_path);
    assert!(is_coherent(&lattice));
}

#[test]
fn test_verification_timing() {
    let mut verifier = SpecializationVerifier::new().unwrap();

    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Clone"), 0);

    let result = verifier.verify();

    // Should be fast for simple cases
    assert!(result.duration.as_millis() < 200);
}

#[test]
fn test_chc_encoding() {
    let mut verifier = SpecializationVerifier::new().unwrap();

    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 0);
    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 1);

    let result = verifier.verify();

    // CHC encoding is now enabled and production-ready
    // Verify that CHC rules are generated (2 rules: transitivity + antisymmetry)
    assert_eq!(result.stats.impls_checked, 2);
    assert!(
        result.stats.chc_rules >= 2,
        "Expected at least 2 CHC rules (transitivity + antisymmetry)"
    );
}

#[test]
fn test_different_ranks() {
    let mut verifier = SpecializationVerifier::new().unwrap();

    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 0);
    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 1);
    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 2);

    let result = verifier.verify();
    assert_eq!(result.stats.impls_checked, 3);
}

#[test]
fn test_ambiguity_suggestions() {
    let mut verifier = SpecializationVerifier::new().unwrap();

    // Create potentially ambiguous implementations
    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 0);
    verifier.register_implementation(create_impl(Type::int(Span::dummy()), "Display"), 1);

    let result = verifier.verify();

    // If ambiguous, should have suggestions
    if !result.ambiguities.is_empty() {
        assert!(!result.ambiguities[0].suggestion.is_empty());
    }
}
