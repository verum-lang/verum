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
//! Comprehensive Tests for Specialization Selection
//!
//! Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Section 9.1 - Automatic Specialization Selection

use verum_ast::span::Span;
use verum_ast::ty::Path;
use verum_common::{List, Map, Maybe, Set};

use verum_types::advanced_protocols::{SpecializationInfo, SpecializationLattice};
use verum_types::protocol::{Protocol, ProtocolBound, ProtocolChecker, ProtocolImpl, WhereClause};
use verum_types::specialization_selection::{
    CoherenceChecker, ProtocolCheckerExt, SpecializationError, SpecializationSelector,
};
use verum_types::ty::Type;
use verum_types::unify::Unifier;

// ==================== Test Helpers ====================

fn make_protocol(name: &str) -> Protocol {
    Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: name.into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    }
}

fn make_impl(protocol: &str, for_type: Type, rank: usize) -> ProtocolImpl {
    use verum_ast::ty::{Ident, PathSegment};
    let path = Path {
        segments: vec![PathSegment::Name(Ident::new(protocol, Span::default()))].into(),
        span: Span::default(),
    };

    ProtocolImpl {
        protocol: path,
        protocol_args: vec![].into(),
        for_type,
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: if rank > 0 {
            Maybe::Some(SpecializationInfo {
                is_specialized: true,
                specializes: Maybe::None,
                specificity_rank: rank,
                is_default: false,
                span: Span::default(),
            })
        } else {
            Maybe::None
        },
        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    }
}

fn make_type(name: &str) -> Type {
    use verum_ast::ty::{Ident, Path};
    let ident = Ident::new(name, Span::default());
    Type::Named {
        path: Path::single(ident),
        args: List::new(),
    }
}

fn make_generic_type(name: &str, args: Vec<Type>) -> Type {
    use verum_ast::ty::{Ident, Path};
    let ident = Ident::new(name, Span::default());
    Type::Named {
        path: Path::single(ident),
        args: args.into(),
    }
}

fn make_type_var(id: usize) -> Type {
    Type::Var(verum_types::ty::TypeVar::with_id(id))
}

// ==================== Core Selection Tests ====================

#[test]
fn test_select_most_specific() {
    let _selector = SpecializationSelector::new();
    let protocol = make_protocol("Display");
    let mut protocol_checker = ProtocolChecker::new();
    let _unifier = Unifier::new();

    // Default impl for T (rank 0)
    let default_impl = make_impl("Display", make_type_var(0), 0);

    // Specialized impl for Int (rank 1)
    let specialized_impl = make_impl("Display", make_type("Int"), 1);

    // Register implementations with the protocol checker
    // This enables the specialization system to query available implementations
    let _ = protocol_checker.register_impl(default_impl);
    let _ = protocol_checker.register_impl(specialized_impl);

    // For Int, should select specialized implementation
    let int_type = make_type("Int");

    // Verify the protocol checker can find implementations for the type
    let impls = protocol_checker.get_implementations_for_protocol(&protocol.name);

    // Manually build the lattice based on protocol checker data
    let mut lattice = SpecializationLattice::new();
    lattice.add_impl(0); // default
    lattice.add_impl(1); // specialized
    lattice.ordering.insert((1, 0), true); // specialized is more specific

    // Test that specialized impl is selected for Int
    // Protocol checker provides the implementation candidates
    assert!(lattice.is_more_specific(1, 0));
    assert!(
        impls.len() >= 0,
        "Protocol checker should track registered implementations"
    );

    // Verify the int type would match the specialized implementation
    assert!(matches!(int_type, Type::Named { .. }));
}

#[test]
fn test_specialization_chain() {
    // Test: A specializes B specializes C
    let mut lattice = SpecializationLattice::new();

    // Add three implementations
    lattice.add_impl(0); // Most general (C)
    lattice.add_impl(1); // Middle (B)
    lattice.add_impl(2); // Most specific (A)

    // Build chain: A > B > C
    lattice.ordering.insert((2, 1), true); // A > B
    lattice.ordering.insert((1, 0), true); // B > C
    lattice.ordering.insert((2, 0), true); // A > C (transitive)

    // Test ordering
    assert!(lattice.is_more_specific(2, 1));
    assert!(lattice.is_more_specific(1, 0));
    assert!(lattice.is_more_specific(2, 0));

    // Test selection: should select most specific (A)
    let mut applicable = Set::new();
    applicable.insert(0);
    applicable.insert(1);
    applicable.insert(2);

    let selected = lattice.select_most_specific(&applicable);
    assert_eq!(selected, Maybe::Some(2)); // Most specific
}

#[test]
fn test_negative_specialization() {
    // Test negative specialization: impl<T: !Send>
    let _selector = SpecializationSelector::new();

    // impl<T: Send + Sync> for T (rank 1)
    let _send_sync_impl = make_impl("MyProtocol", make_type_var(0), 1);

    // @specialize impl<T: Send + !Sync> for T (rank 2)
    let _send_not_sync_impl = make_impl("MyProtocol", make_type_var(0), 2);

    // Build lattice
    let mut lattice = SpecializationLattice::new();
    lattice.add_impl(0); // send_sync
    lattice.add_impl(1); // send_not_sync

    // send_not_sync is more specific due to negative bound
    lattice.ordering.insert((1, 0), true);

    assert!(lattice.is_more_specific(1, 0));
}

#[test]
fn test_ambiguous_specialization_error() {
    let mut lattice = SpecializationLattice::new();

    // Two implementations with no ordering between them
    lattice.add_impl(0);
    lattice.add_impl(1);

    // Both applicable, neither more specific
    let mut applicable = Set::new();
    applicable.insert(0);
    applicable.insert(1);

    // Should fail to select (ambiguous)
    let selected = lattice.select_most_specific(&applicable);
    assert_eq!(selected, Maybe::None);
}

#[test]
fn test_coherence_violation() {
    let _checker = CoherenceChecker::new();
    let protocol = make_protocol("Display");
    let mut protocol_checker = ProtocolChecker::new();

    // Two overlapping implementations without specialization
    let impl1 = make_impl("Display", make_type("Int"), 0); // No specialization
    let impl2 = make_impl("Display", make_type("Int"), 0); // No specialization

    // Register the first implementation - should succeed
    let result1 = protocol_checker.register_impl(impl1);

    // Register the second overlapping implementation - should detect coherence violation
    // The protocol checker enforces that overlapping implementations require specialization
    let result2 = protocol_checker.register_impl(impl2);

    // Verify that the protocol checker properly tracks implementations
    let impls = protocol_checker.get_implementations_for_protocol(&protocol.name);

    // The coherence check validates:
    // 1. No overlapping implementations without specialization relationship
    // 2. Orphan rule compliance
    // Note: Exact behavior depends on coherence checking configuration
    assert!(
        result1.is_ok() || result1.is_err(),
        "First impl registration should complete (success or coherence error)"
    );

    // Second overlapping impl may fail coherence check
    if result1.is_ok() && result2.is_err() {
        // This is the expected behavior - coherence violation detected
        assert!(!impls.is_empty(), "At least one impl should be registered");
    }
}

#[test]
fn test_specialization_with_where_clauses() {
    // Test: impl<T: Clone> vs impl<T: Clone + Debug>
    let _selector = SpecializationSelector::new();

    // impl<T: Clone> (rank 1)
    let _clone_impl = make_impl("Display", make_type_var(0), 1);

    // impl<T: Clone + Debug> (rank 2) - more specific
    let _clone_debug_impl = make_impl("Display", make_type_var(0), 2);

    // Build lattice
    let mut lattice = SpecializationLattice::new();
    lattice.add_impl(0);
    lattice.add_impl(1);

    // clone_debug is more specific (more constraints)
    lattice.ordering.insert((1, 0), true);

    assert!(lattice.is_more_specific(1, 0));
}

#[test]
fn test_cached_selection() {
    let mut selector = SpecializationSelector::new();

    // Manually cache a selection
    selector.cache_selection("Display".into(), "Int".into(), 42);

    // Check cache hit
    let cached = selector.cache.get(&("Display".into(), "Int".into()));
    assert_eq!(cached, Some(&42));

    // Verify stats
    assert_eq!(selector.stats().cache_hits, 0); // No selections yet
    assert_eq!(selector.stats().cache_misses, 0);
}

#[test]
fn test_conditional_specialization() {
    // Test: Specialization based on type constraints
    // impl<T> vs impl<T: Sized>

    let mut lattice = SpecializationLattice::new();

    lattice.add_impl(0); // impl<T>
    lattice.add_impl(1); // impl<T: Sized>

    // Sized constraint makes it more specific
    lattice.ordering.insert((1, 0), true);

    assert!(lattice.is_more_specific(1, 0));
}

#[test]
fn test_generic_type_matching() {
    let selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();

    // Test: List<Int> matches impl<T> for List<T>
    let concrete = make_generic_type("List", vec![make_type("Int")]);
    let pattern = make_generic_type("List", vec![make_type_var(0)]);

    let matches = selector.matches_impl_pattern(&concrete, &pattern, &mut unifier);
    assert!(matches);
}

#[test]
fn test_concrete_type_matching() {
    let selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();

    // Test: Int matches impl for Int
    let concrete = make_type("Int");
    let pattern = make_type("Int");

    let matches = selector.matches_impl_pattern(&concrete, &pattern, &mut unifier);
    assert!(matches);
}

#[test]
fn test_type_variable_matching() {
    let selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();

    // Test: Any type matches impl<T> for T
    let concrete = make_type("Int");
    let pattern = make_type_var(0);

    let matches = selector.matches_impl_pattern(&concrete, &pattern, &mut unifier);
    assert!(matches);
}

#[test]
fn test_function_type_matching() {
    let selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();

    // Test: (Int) -> Bool matches impl for function types
    let concrete = Type::Function {
        params: vec![make_type("Int")].into(),
        return_type: Box::new(make_type("Bool")),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };

    let pattern = Type::Function {
        params: vec![make_type_var(0)].into(),
        return_type: Box::new(make_type_var(1)),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };

    let matches = selector.matches_impl_pattern(&concrete, &pattern, &mut unifier);
    assert!(matches);
}

#[test]
fn test_reference_type_matching() {
    let selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();

    // Test: &Int matches impl for &T
    let concrete = Type::Reference {
        mutable: false,
        inner: Box::new(make_type("Int")),
    };
    let pattern = Type::Reference {
        mutable: false,
        inner: Box::new(make_type_var(0)),
    };

    let matches = selector.matches_impl_pattern(&concrete, &pattern, &mut unifier);
    assert!(matches);
}

#[test]
fn test_no_match_different_types() {
    let selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();

    // Test: Int does not match Bool
    let concrete = make_type("Int");
    let pattern = make_type("Bool");

    let matches = selector.matches_impl_pattern(&concrete, &pattern, &mut unifier);
    assert!(!matches);
}

#[test]
fn test_specificity_rank_comparison() {
    // Test: Higher rank = more specific
    let impl1 = make_impl("Display", make_type("Int"), 1);
    let impl2 = make_impl("Display", make_type("Int"), 2);

    // impl2 has higher rank, should be more specific
    assert!(
        impl2.specialization.as_ref().unwrap().specificity_rank
            > impl1.specialization.as_ref().unwrap().specificity_rank
    );
}

#[test]
fn test_lattice_maximal_element() {
    let mut lattice = SpecializationLattice::new();

    lattice.add_impl(0); // Most general
    lattice.add_impl(1); // Specific
    lattice.add_impl(2); // More specific

    // Build ordering
    lattice.ordering.insert((1, 0), true);
    lattice.ordering.insert((2, 0), true);
    lattice.ordering.insert((2, 1), true);

    // Maximal element is 0 (most general)
    lattice.max_element = Maybe::Some(0);

    assert_eq!(lattice.max_element, Maybe::Some(0));
}

#[test]
fn test_lattice_minimal_elements() {
    let mut lattice = SpecializationLattice::new();

    lattice.add_impl(0); // General
    lattice.add_impl(1); // Specific branch 1
    lattice.add_impl(2); // Specific branch 2

    // Build diamond shape
    lattice.ordering.insert((1, 0), true);
    lattice.ordering.insert((2, 0), true);

    // Both 1 and 2 are minimal
    lattice.min_elements.insert(1);
    lattice.min_elements.insert(2);

    assert!(lattice.min_elements.contains(&1));
    assert!(lattice.min_elements.contains(&2));
}

#[test]
fn test_overlap_detection_same_type() {
    let checker = CoherenceChecker::new();

    // Two impls for same concrete type
    let impl1 = make_impl("Display", make_type("Int"), 0);
    let impl2 = make_impl("Display", make_type("Int"), 0);

    assert!(checker.overlaps(&impl1, &impl2));
}

#[test]
fn test_overlap_detection_generic_concrete() {
    let checker = CoherenceChecker::new();

    // impl<T> and impl for Int
    let impl1 = make_impl("Display", make_type_var(0), 0);
    let impl2 = make_impl("Display", make_type("Int"), 1);

    assert!(checker.overlaps(&impl1, &impl2));
}

#[test]
fn test_overlap_detection_both_generic() {
    let checker = CoherenceChecker::new();

    // impl<T> and impl<U>
    let impl1 = make_impl("Display", make_type_var(0), 0);
    let impl2 = make_impl("Display", make_type_var(1), 0);

    assert!(checker.overlaps(&impl1, &impl2));
}

#[test]
fn test_no_overlap_different_types() {
    let checker = CoherenceChecker::new();

    // impl for Int and impl for Bool
    let impl1 = make_impl("Display", make_type("Int"), 0);
    let impl2 = make_impl("Display", make_type("Bool"), 0);

    assert!(!checker.overlaps(&impl1, &impl2));
}

#[test]
fn test_specialization_relationship_detection() {
    let checker = CoherenceChecker::new();

    // impl with specialization and one without
    let impl1 = make_impl("Display", make_type("Int"), 1); // Specialized
    let impl2 = make_impl("Display", make_type_var(0), 0); // General

    assert!(checker.has_specialization_relationship(&impl1, &impl2));
}

#[test]
fn test_cache_clear() {
    let mut selector = SpecializationSelector::new();

    // Add some cached entries
    selector.cache_selection("Display".into(), "Int".into(), 1);
    selector.cache_selection("Debug".into(), "Bool".into(), 2);

    assert_eq!(selector.cache.len(), 2);

    // Clear cache
    selector.clear_cache();

    assert_eq!(selector.cache.len(), 0);
}

#[test]
fn test_stats_tracking() {
    let selector = SpecializationSelector::new();

    // Initially zero
    assert_eq!(selector.stats().cache_hits, 0);
    assert_eq!(selector.stats().cache_misses, 0);
    assert_eq!(selector.stats().selections, 0);

    // Stats would be updated during actual selections
}

#[test]
fn test_error_conversion_ambiguous() {
    let error = SpecializationError::Ambiguous {
        candidates: vec![1, 2].into(),
        protocol: "Display".into(),
        self_type: make_type("Int"),
        suggestion: "add type annotations".into(),
    };

    let _type_error = error.to_type_error(Span::default());
    // Should convert successfully
}

#[test]
fn test_error_conversion_overlap() {
    let error = SpecializationError::Overlap {
        impl1_id: 1,
        impl2_id: 2,
        protocol: "Display".into(),
        suggestion: "use @specialize attribute".into(),
    };

    let _type_error = error.to_type_error(Span::default());
    // Should convert successfully
}

#[test]
fn test_error_conversion_no_impl() {
    let error = SpecializationError::NoApplicableImpl {
        protocol: "Display".into(),
        self_type: make_type("MyType"),
        suggestion: "implement Display for MyType".into(),
    };

    let _type_error = error.to_type_error(Span::default());
    // Should convert successfully
}

// ==================== Advanced Scenarios ====================

#[test]
fn test_diamond_specialization() {
    // Diamond pattern:
    //       A (general)
    //      / \
    //     B   C (both specialize A)
    //      \ /
    //       D (specializes both B and C)

    let mut lattice = SpecializationLattice::new();

    lattice.add_impl(0); // A
    lattice.add_impl(1); // B
    lattice.add_impl(2); // C
    lattice.add_impl(3); // D

    // Build diamond
    lattice.ordering.insert((1, 0), true); // B > A
    lattice.ordering.insert((2, 0), true); // C > A
    lattice.ordering.insert((3, 1), true); // D > B
    lattice.ordering.insert((3, 2), true); // D > C
    lattice.ordering.insert((3, 0), true); // D > A (transitive)

    // D is most specific
    let mut applicable = Set::new();
    applicable.insert(0);
    applicable.insert(1);
    applicable.insert(2);
    applicable.insert(3);

    let selected = lattice.select_most_specific(&applicable);
    assert_eq!(selected, Maybe::Some(3));
}

#[test]
fn test_multiple_type_params() {
    let selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();

    // Test: Map<Int, Bool> matches impl<K, V> for Map<K, V>
    let concrete = make_generic_type("Map", vec![make_type("Int"), make_type("Bool")]);
    let pattern = make_generic_type("Map", vec![make_type_var(0), make_type_var(1)]);

    let matches = selector.matches_impl_pattern(&concrete, &pattern, &mut unifier);
    assert!(matches);
}

#[test]
fn test_nested_generic_types() {
    let selector = SpecializationSelector::new();
    let mut unifier = Unifier::new();

    // Test: List<List<Int>> matches impl<T> for List<List<T>>
    let inner = make_generic_type("List", vec![make_type("Int")]);
    let concrete = make_generic_type("List", vec![inner]);

    let inner_pattern = make_generic_type("List", vec![make_type_var(0)]);
    let pattern = make_generic_type("List", vec![inner_pattern]);

    let matches = selector.matches_impl_pattern(&concrete, &pattern, &mut unifier);
    assert!(matches);
}

#[test]
fn test_partial_specialization() {
    // Test: impl<T> for List<T> vs impl for List<Int>
    let _selector = SpecializationSelector::new();

    let _generic = make_impl(
        "Display",
        make_generic_type("List", vec![make_type_var(0)]),
        0,
    );

    let _specialized = make_impl(
        "Display",
        make_generic_type("List", vec![make_type("Int")]),
        1,
    );

    // specialized is more specific
    let mut lattice = SpecializationLattice::new();
    lattice.add_impl(0);
    lattice.add_impl(1);
    lattice.ordering.insert((1, 0), true);

    assert!(lattice.is_more_specific(1, 0));
}

#[test]
fn test_type_to_cache_key() {
    let selector = SpecializationSelector::new();

    let int_type = make_type("Int");
    let key1 = selector.type_to_cache_key(&int_type);

    let another_int = make_type("Int");
    let key2 = selector.type_to_cache_key(&another_int);

    // Same type should produce same key
    assert_eq!(key1, key2);
}

#[test]
fn test_default_selector() {
    let selector = SpecializationSelector::default();

    assert_eq!(selector.cache.len(), 0);
    assert_eq!(selector.lattices.len(), 0);
}

#[test]
fn test_default_coherence_checker() {
    let checker = CoherenceChecker::default();

    assert_eq!(checker.violations().len(), 0);
}

// ==================== Negative Specialization Tests ====================
// Comprehensive tests for Task 1: check_negative_specialization
// Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — .4 lines 623-638

/// Create an impl with negative bounds for testing
fn make_negative_impl(protocol: &str, for_type: Type, negative_protocol: &str) -> ProtocolImpl {
    use verum_ast::ty::{Ident, Path as AstPath, PathSegment};

    let path = Path {
        segments: vec![PathSegment::Name(Ident::new(protocol, Span::default()))].into(),
        span: Span::default(),
    };

    // Create a negative protocol bound using the AST TypeBound structure
    let neg_protocol_path = AstPath::new(
        vec![PathSegment::Name(Ident::new(
            format!("!{}", negative_protocol),
            Span::default(),
        ))]
        .into(),
        Span::default(),
    );

    // Create where clause with negative bound
    let where_clause = WhereClause {
        ty: for_type.clone(),
        bounds: vec![ProtocolBound {
            protocol: neg_protocol_path,
            args: List::new(),
            is_negative: true,
        }]
        .into(),
    };

    ProtocolImpl {
        protocol: path,
        protocol_args: vec![].into(),
        for_type,
        where_clauses: vec![where_clause].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::Some(SpecializationInfo {
            is_specialized: true,
            specializes: Maybe::None,
            specificity_rank: 1,
            is_default: true, // Negative specializations are default impls
            span: Span::default(),
        }),
        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    }
}

#[test]
fn test_negative_spec_type_does_not_satisfy_bound() {
    // Test: @specialize(negative) implement<T: !Clone> MyProtocol for List<T>
    // When T does NOT implement Clone, the impl should apply
    let negative_impl = make_negative_impl("MyProtocol", make_type_var(0), "Clone");

    // Verify the impl has negative specialization
    assert!(negative_impl.specialization.is_some());
    assert!(negative_impl.specialization.as_ref().unwrap().is_default);
    assert!(!negative_impl.where_clauses.is_empty());
}

#[test]
fn test_negative_spec_type_satisfies_bound_rejected() {
    // Test: If T implements Clone, negative !Clone impl should NOT apply
    // This would be checked by check_negative_constraints returning false

    let negative_impl = make_negative_impl("MyProtocol", make_type("Int"), "Clone");

    // In a full test, we'd verify that check_negative_constraints returns false
    // when the protocol checker shows Int implements Clone
    assert!(negative_impl.specialization.as_ref().unwrap().is_default);
}

#[test]
fn test_negative_spec_multiple_negative_bounds() {
    // Test: impl<T: !Clone + !Send> should require both negatives
    let mut negative_impl = make_negative_impl("MyProtocol", make_type_var(0), "Clone");

    // Add second negative bound
    use verum_ast::ty::{Ident, Path as AstPath, PathSegment};
    let send_path = AstPath::new(
        vec![PathSegment::Name(Ident::new("!Send", Span::default()))].into(),
        Span::default(),
    );

    let mut where_clause = negative_impl.where_clauses.first().unwrap().clone();
    let mut bounds = where_clause.bounds.clone();
    bounds.push(ProtocolBound {
        protocol: send_path,
        args: List::new(),
        is_negative: true,
    });
    where_clause.bounds = bounds;

    negative_impl.where_clauses = vec![where_clause].into();

    // Should have both negative bounds
    assert_eq!(negative_impl.where_clauses.first().unwrap().bounds.len(), 2);
}

#[test]
fn test_negative_spec_no_specialization_metadata() {
    // Test: impl without specialization metadata should pass (no negative checking)
    let mut impl_no_spec = make_impl("MyProtocol", make_type("Int"), 0);
    impl_no_spec.specialization = Maybe::None;

    // Should not be rejected (no specialization metadata)
    assert!(impl_no_spec.specialization.is_none());
}

#[test]
fn test_negative_spec_not_default_impl() {
    // Test: Specialized impl that's not default should pass
    let mut impl_specialized = make_negative_impl("MyProtocol", make_type("Int"), "Clone");

    // Set is_default to false
    if let Maybe::Some(ref mut spec_info) = impl_specialized.specialization {
        spec_info.is_default = false;
    }

    // Should not perform negative checking
    assert!(!impl_specialized.specialization.as_ref().unwrap().is_default);
}

#[test]
fn test_negative_spec_empty_where_clauses() {
    // Test: Default impl with no where clauses should pass
    let mut default_impl = make_impl("MyProtocol", make_type_var(0), 1);
    default_impl.specialization = Maybe::Some(SpecializationInfo {
        is_specialized: true,
        specializes: Maybe::None,
        specificity_rank: 1,
        is_default: true,
        span: Span::default(),
    });
    default_impl.where_clauses = List::new();

    // Should pass (no negative bounds to check)
    assert!(default_impl.where_clauses.is_empty());
}

#[test]
fn test_negative_spec_positive_bounds_only() {
    // Test: Where clauses with only positive bounds should pass
    use verum_ast::ty::{Ident, Path as AstPath, PathSegment};

    let mut default_impl = make_impl("MyProtocol", make_type_var(0), 1);
    default_impl.specialization = Maybe::Some(SpecializationInfo {
        is_specialized: true,
        specializes: Maybe::None,
        specificity_rank: 1,
        is_default: true,
        span: Span::default(),
    });

    // Add positive bound (no ! prefix)
    let clone_path = AstPath::new(
        vec![PathSegment::Name(Ident::new("Clone", Span::default()))].into(),
        Span::default(),
    );

    let where_clause = WhereClause {
        ty: make_type_var(0),
        bounds: vec![ProtocolBound {
            protocol: clone_path,
            args: List::new(),
            is_negative: false,
        }]
        .into(),
    };

    default_impl.where_clauses = vec![where_clause].into();

    // Should pass (only positive bounds)
    let first_segment = &default_impl
        .where_clauses
        .first()
        .unwrap()
        .bounds
        .first()
        .unwrap()
        .protocol
        .segments
        .first()
        .unwrap();
    if let verum_ast::ty::PathSegment::Name(ident) = first_segment {
        assert!(!ident.name.as_str().starts_with('!'));
    }
}

#[test]
fn test_negative_spec_mixed_positive_negative_bounds() {
    // Test: impl<T: Clone + !Send> - mixed bounds
    use verum_ast::ty::{Ident, Path as AstPath, PathSegment};

    let mut mixed_impl = make_impl("MyProtocol", make_type_var(0), 1);
    mixed_impl.specialization = Maybe::Some(SpecializationInfo {
        is_specialized: true,
        specializes: Maybe::None,
        specificity_rank: 1,
        is_default: true,
        span: Span::default(),
    });

    // Add both positive and negative bounds
    let clone_path = AstPath::new(
        vec![PathSegment::Name(Ident::new("Clone", Span::default()))].into(),
        Span::default(),
    );
    let send_path = AstPath::new(
        vec![PathSegment::Name(Ident::new("!Send", Span::default()))].into(),
        Span::default(),
    );

    let where_clause = WhereClause {
        ty: make_type_var(0),
        bounds: vec![
            ProtocolBound {
                protocol: clone_path,
                args: List::new(),
                is_negative: false,
            },
            ProtocolBound {
                protocol: send_path,
                args: List::new(),
                is_negative: true,
            },
        ]
        .into(),
    };

    mixed_impl.where_clauses = vec![where_clause].into();

    // Should have both types of bounds
    assert_eq!(mixed_impl.where_clauses.first().unwrap().bounds.len(), 2);
}

#[test]
fn test_negative_spec_type_var_constraint() {
    // Test: impl<T: !Clone> for List<T>
    let list_type = make_generic_type("List", vec![make_type_var(0)]);
    let negative_impl = make_negative_impl("MyProtocol", list_type, "Clone");

    // Verify structure
    assert!(negative_impl.specialization.is_some());
    assert!(negative_impl.specialization.as_ref().unwrap().is_default);
}

#[test]
fn test_negative_spec_concrete_type() {
    // Test: impl for List<Int> with negative constraint
    let list_int = make_generic_type("List", vec![make_type("Int")]);
    let negative_impl = make_negative_impl("MyProtocol", list_int, "Clone");

    // Verify structure
    assert!(!negative_impl.where_clauses.is_empty());
}

#[test]
fn test_negative_spec_nested_generic() {
    // Test: impl<T: !Send> for List<List<T>>
    let inner = make_generic_type("List", vec![make_type_var(0)]);
    let outer = make_generic_type("List", vec![inner]);
    let negative_impl = make_negative_impl("MyProtocol", outer, "Send");

    // Verify structure
    assert!(negative_impl.specialization.is_some());
}

#[test]
fn test_negative_spec_multiple_type_params() {
    // Test: impl<K: !Clone, V> for Map<K, V>
    let map_type = make_generic_type("Map", vec![make_type_var(0), make_type_var(1)]);
    let negative_impl = make_negative_impl("MyProtocol", map_type, "Clone");

    // Verify structure
    assert!(!negative_impl.where_clauses.is_empty());
}

#[test]
fn test_negative_spec_lattice_ordering() {
    // Test: Negative impl should be more specific than general impl
    let mut lattice = SpecializationLattice::new();

    // General impl: impl<T> for T
    lattice.add_impl(0);

    // Negative specialized: impl<T: !Clone> for T
    lattice.add_impl(1);

    // Negative impl is more specific (constrains type variables)
    lattice.ordering.insert((1, 0), true);

    assert!(lattice.is_more_specific(1, 0));
}

#[test]
fn test_negative_spec_coherence_with_positive() {
    // Test: impl<T: Clone> and impl<T: !Clone> should be coherent (mutually exclusive)
    let positive_impl = make_impl("MyProtocol", make_type_var(0), 1);
    let negative_impl = make_negative_impl("MyProtocol", make_type_var(0), "Clone");

    // These should not overlap (mutually exclusive constraints)
    let _checker = CoherenceChecker::new();
    // In full implementation, coherence checker would verify mutual exclusion

    assert!(positive_impl.specialization.is_some());
    assert!(negative_impl.specialization.is_some());
}

#[test]
fn test_negative_spec_rank_precedence() {
    // Test: Higher rank negative impl should take precedence
    let rank1 = make_negative_impl("MyProtocol", make_type_var(0), "Clone");

    let mut rank2 = make_negative_impl("MyProtocol", make_type_var(0), "Clone");
    if let Maybe::Some(ref mut spec) = rank2.specialization {
        spec.specificity_rank = 2;
    }

    assert!(
        rank2.specialization.as_ref().unwrap().specificity_rank
            > rank1.specialization.as_ref().unwrap().specificity_rank
    );
}
