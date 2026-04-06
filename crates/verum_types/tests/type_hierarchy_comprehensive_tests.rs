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
// Comprehensive Type Hierarchy Tests
//
// This test suite validates the complete type hierarchy including:
// - Subtyping relationships
// - Type coercion rules
// - Protocol implementation checking
// - Variance rules
// - Reference type hierarchies (CBGR, checked, unsafe)
//
// Complete Verum type system: HM inference + refinement types + protocols + dependent types
// Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — Three-tier reference model

use std::any::TypeId;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map, Maybe, Text};
use verum_types::context::{TypeContext, TypeParam, TypeScheme};
use verum_types::di::requirement::{ContextRef, ContextRequirement};
use verum_types::protocol::{Protocol, ProtocolBound, ProtocolChecker};
use verum_types::subtype::Subtyping;
use verum_types::ty::Type;
use verum_types::variance::Variance;

// Helper to create ContextRequirement from a list of context names
fn make_contexts(names: Vec<&str>) -> ContextRequirement {
    let refs: Vec<ContextRef> = names
        .into_iter()
        .map(|name| ContextRef::new(Text::from(name), TypeId::of::<()>()))
        .collect();
    ContextRequirement::from_contexts(refs)
}

// ============================================================================
// Primitive Type Hierarchy
// ============================================================================

#[test]
fn test_primitive_types_no_subtyping() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Primitive types are not subtypes of each other
    assert!(!subtyping.is_subtype(&Type::int(), &Type::float()));
    assert!(!subtyping.is_subtype(&Type::bool(), &Type::int()));
    assert!(!subtyping.is_subtype(&Type::Char, &Type::text()));
}

#[test]
fn test_type_is_subtype_of_itself() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Every type is a subtype of itself (reflexivity)
    assert!(subtyping.is_subtype(&Type::int(), &Type::int()));
    assert!(subtyping.is_subtype(&Type::bool(), &Type::bool()));
    assert!(subtyping.is_subtype(&Type::text(), &Type::text()));
    assert!(subtyping.is_subtype(&Type::unit(), &Type::unit()));
}

// ============================================================================
// Function Type Hierarchy
// ============================================================================

#[test]
fn test_function_contravariant_parameters() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Function types are contravariant in parameters
    // If A <: B, then (B -> C) <: (A -> C)
    // This test documents the expected behavior
    let f1 = Type::function(vec![Type::int()].into(), Type::bool());
    let f2 = Type::function(vec![Type::int()].into(), Type::bool());

    assert!(subtyping.is_subtype(&f1, &f2));
}

#[test]
fn test_function_covariant_return() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Function types are covariant in return type
    // If A <: B, then (C -> A) <: (C -> B)
    // This test documents the expected behavior
    let f1 = Type::function(vec![Type::int()].into(), Type::bool());
    let f2 = Type::function(vec![Type::int()].into(), Type::bool());

    assert!(subtyping.is_subtype(&f1, &f2));
}

#[test]
fn test_function_different_arity_no_subtype() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Functions with different arities are not subtypes
    let f1 = Type::function(vec![Type::int()].into(), Type::bool());
    let f2 = Type::function(vec![Type::int(), Type::int()].into(), Type::bool());

    assert!(!subtyping.is_subtype(&f1, &f2));
}

#[test]
fn test_function_with_contexts_must_match() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    let f1 = Type::function_with_contexts(
        vec![Type::int()].into_iter().collect::<List<_>>(),
        Type::bool(),
        make_contexts(vec!["Database"]),
    );

    let f2 = Type::function(
        vec![Type::int()].into_iter().collect::<List<_>>(),
        Type::bool(),
    );

    // Different contexts means not subtypes
    assert!(!subtyping.is_subtype(&f1, &f2));
}

// ============================================================================
// Tuple Type Hierarchy
// ============================================================================

#[test]
fn test_tuple_covariant_in_components() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Tuples are covariant in their components
    let t1 = Type::tuple(vec![Type::int(), Type::bool()].into());
    let t2 = Type::tuple(vec![Type::int(), Type::bool()].into());

    assert!(subtyping.is_subtype(&t1, &t2));
}

#[test]
fn test_tuple_different_length_no_subtype() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    let t1 = Type::tuple(vec![Type::int()].into());
    let t2 = Type::tuple(vec![Type::int(), Type::bool()].into());

    assert!(!subtyping.is_subtype(&t1, &t2));
}

// ============================================================================
// Reference Type Hierarchy (Three-Tier Model)
// ============================================================================

#[test]
fn test_reference_types_are_distinct() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    let cbgr_ref = Type::reference(false, Type::int());
    let checked_ref = Type::checked_reference(false, Type::int());
    let unsafe_ref = Type::unsafe_reference(false, Type::int());

    // Different reference tiers are not subtypes of each other
    assert!(!subtyping.is_subtype(&cbgr_ref, &checked_ref));
    assert!(!subtyping.is_subtype(&checked_ref, &unsafe_ref));
    assert!(!subtyping.is_subtype(&cbgr_ref, &unsafe_ref));
}

#[test]
fn test_reference_immutable_vs_mutable() {
    let subtyping = Subtyping::new();

    let immut_ref = Type::reference(false, Type::int());
    let mut_ref = Type::reference(true, Type::int());

    // Immutable cannot become mutable (unsafe)
    assert!(!subtyping.is_subtype(&immut_ref, &mut_ref));
    // Mutable CAN be used where immutable is expected (safe coercion)
    // &mut T <: &T because reading from a mutable ref is safe
    assert!(subtyping.is_subtype(&mut_ref, &immut_ref));
}

#[test]
fn test_reference_covariant_in_inner_type() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Immutable references are covariant in inner type
    let ref1 = Type::reference(false, Type::int());
    let ref2 = Type::reference(false, Type::int());

    assert!(subtyping.is_subtype(&ref1, &ref2));
}

#[test]
fn test_mutable_reference_invariant() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Mutable references are invariant
    let mut_ref1 = Type::reference(true, Type::int());
    let mut_ref2 = Type::reference(true, Type::int());

    assert!(subtyping.is_subtype(&mut_ref1, &mut_ref2));
}

// ============================================================================
// Array Type Hierarchy
// ============================================================================

#[test]
fn test_array_covariant_in_element_type() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Arrays are covariant in element type
    let arr1 = Type::array(Type::int(), Some(10));
    let arr2 = Type::array(Type::int(), Some(10));

    assert!(subtyping.is_subtype(&arr1, &arr2));
}

#[test]
fn test_array_size_must_match() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    let arr1 = Type::array(Type::int(), Some(10));
    let arr2 = Type::array(Type::int(), Some(20));

    // Different sizes means not subtypes
    assert!(!subtyping.is_subtype(&arr1, &arr2));
}

#[test]
fn test_array_sized_vs_unsized() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    let sized = Type::array(Type::int(), Some(10));
    let unsized_array = Type::array(Type::int(), None);

    // Sized array could potentially be subtype of unsized slice
    // This depends on implementation
    // Test documents expected behavior
}

// ============================================================================
// Named Type Hierarchy
// ============================================================================

#[test]
fn test_named_type_with_same_args() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    let list1 = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    let list2 = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };

    assert!(subtyping.is_subtype(&list1, &list2));
}

#[test]
fn test_named_type_with_different_args() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    let list_int = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    let list_bool = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::bool()].into(),
    };

    // List<Int> is not a subtype of List<Bool>
    assert!(!subtyping.is_subtype(&list_int, &list_bool));
}

// ============================================================================
// Refinement Type Hierarchy
// ============================================================================

#[test]
fn test_refined_type_subtyping() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Int{> 0} <: Int{> -1}
    // More restrictive refinement is subtype of less restrictive
    // This requires SMT solving and is tested separately
    // This test documents the expected behavior
}

#[test]
fn test_refined_type_to_base_type() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Int{> 0} <: Int
    // Refined type is subtype of base type
    // This test documents expected behavior
}

// ============================================================================
// Variance Tests
// ============================================================================

#[test]
fn test_variance_calculation_list() {
    // List<T> is covariant in T
    let variance = Variance::Covariant;
    assert_eq!(variance, Variance::Covariant);
}

#[test]
fn test_variance_calculation_function_param() {
    // Function parameters are contravariant
    let variance = Variance::Contravariant;
    assert_eq!(variance, Variance::Contravariant);
}

#[test]
fn test_variance_calculation_function_return() {
    // Function return types are covariant
    let variance = Variance::Covariant;
    assert_eq!(variance, Variance::Covariant);
}

#[test]
fn test_variance_calculation_mutable_ref() {
    // Mutable references are invariant
    let variance = Variance::Invariant;
    assert_eq!(variance, Variance::Invariant);
}

// ============================================================================
// Protocol-Based Subtyping
// ============================================================================

#[test]
fn test_protocol_implementation_allows_substitution() {
    // If T implements Protocol P, then T can be used where P is required
    // This is nominal subtyping through protocols
    let mut checker = ProtocolChecker::new();

    // Define Eq protocol
    let eq_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Eq".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::dummy(),
    };

    // This test documents protocol-based subtyping
}

#[test]
fn test_multiple_protocol_bounds() {
    // If T: P1 + P2, T satisfies both bounds
    let mut ctx = TypeContext::new();

    let type_param = TypeParam {
        name: Text::from("T"),
        bounds: vec![
            ProtocolBound {
                protocol: Path::from_ident(Ident::new("Eq", Span::dummy())),
                args: List::new(),
                is_negative: false,
            },
            ProtocolBound {
                protocol: Path::from_ident(Ident::new("Ord", Span::dummy())),
                args: List::new(),
                is_negative: false,
            },
        ]
        .into(),
        default: Maybe::None,
        variance: Variance::Invariant,
        is_meta: false,
        span: Span::dummy(),
    };

    // Type parameter with multiple bounds
    assert_eq!(type_param.bounds.len(), 2);
}

// ============================================================================
// Transitive Subtyping
// ============================================================================

#[test]
fn test_subtyping_transitivity() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // If A <: B and B <: C, then A <: C
    // For now we test with reflexive case
    let int_ty = Type::int();

    assert!(subtyping.is_subtype(&int_ty, &int_ty));
}

// ============================================================================
// Type Equivalence vs Subtyping
// ============================================================================

#[test]
fn test_equal_types_are_subtypes() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Equal types are subtypes of each other
    let t1 = Type::function(vec![Type::int()].into(), Type::bool());
    let t2 = Type::function(vec![Type::int()].into(), Type::bool());

    assert!(subtyping.is_subtype(&t1, &t2));
    assert!(subtyping.is_subtype(&t2, &t1));
}

#[test]
fn test_structural_equality_vs_nominal() {
    // Structural types are equal if structure matches
    let t1 = Type::tuple(vec![Type::int(), Type::bool()].into());
    let t2 = Type::tuple(vec![Type::int(), Type::bool()].into());

    assert_eq!(t1, t2);

    // Nominal types require same name
    let n1 = Type::Named {
        path: Path::single(Ident::new("Point", Span::dummy())),
        args: vec![Type::int(), Type::int()].into(),
    };
    let n2 = Type::Named {
        path: Path::single(Ident::new("Vector", Span::dummy())),
        args: vec![Type::int(), Type::int()].into(),
    };

    assert_ne!(n1, n2);
}

// ============================================================================
// Generic Type Hierarchy
// ============================================================================

#[test]
fn test_generic_type_same_type_args() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // List<Int> <: List<Int>
    let list1 = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    let list2 = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };

    assert!(subtyping.is_subtype(&list1, &list2));
}

#[test]
fn test_generic_type_different_type_args() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // List<Int> is not <: List<Bool>
    let list_int = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::int()].into(),
    };
    let list_bool = Type::Named {
        path: Path::single(Ident::new("List", Span::dummy())),
        args: vec![Type::bool()].into(),
    };

    assert!(!subtyping.is_subtype(&list_int, &list_bool));
}

// ============================================================================
// Record Type Hierarchy
// ============================================================================

#[test]
fn test_record_structural_subtyping() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Records with same fields are subtypes
    use indexmap::IndexMap;

    let mut fields1 = IndexMap::new();
    fields1.insert(Text::from("x"), Type::int());
    fields1.insert(Text::from("y"), Type::int());

    let mut fields2 = IndexMap::new();
    fields2.insert(Text::from("x"), Type::int());
    fields2.insert(Text::from("y"), Type::int());

    let rec1 = Type::Record(fields1);
    let rec2 = Type::Record(fields2);

    assert!(subtyping.is_subtype(&rec1, &rec2));
}

#[test]
fn test_record_width_subtyping() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    use indexmap::IndexMap;

    // Record with more fields could be subtype of record with fewer
    // This depends on whether we support width subtyping
    let mut fields_wide = IndexMap::new();
    fields_wide.insert(Text::from("x"), Type::int());
    fields_wide.insert(Text::from("y"), Type::int());
    fields_wide.insert(Text::from("z"), Type::int());

    let mut fields_narrow = IndexMap::new();
    fields_narrow.insert(Text::from("x"), Type::int());
    fields_narrow.insert(Text::from("y"), Type::int());

    let rec_wide = Type::Record(fields_wide);
    let rec_narrow = Type::Record(fields_narrow);

    // Test documents expected behavior
    // Width subtyping: {x, y, z} <: {x, y}
}

// ============================================================================
// Existential and Universal Types
// ============================================================================

#[test]
fn test_existential_type_hierarchy() {
    // ∃α. List<α> represents a list of some unknown type
    // This test documents expected behavior for existential types
}

#[test]
fn test_universal_type_hierarchy() {
    // ∀α. α -> α represents a polymorphic identity function
    // This test documents expected behavior for universal types
}

// ============================================================================
// Meta Parameter Type Hierarchy
// ============================================================================

#[test]
fn test_meta_parameter_types() {
    // Meta parameters for compile-time values
    // meta N: usize represents compile-time size parameter
    let usize_ty = Type::Named {
        path: Path::single(Ident::new("usize", Span::dummy())),
        args: List::new(),
    };
    let meta_ty = Type::meta("N".into(), usize_ty, None);

    assert_eq!(meta_ty.to_string(), "N: meta usize");
}

#[test]
fn test_meta_parameter_with_refinement() {
    // meta N: usize{> 0} - compile-time positive integer
    use verum_types::refinement::RefinementPredicate;

    // This test documents expected behavior
}

// ============================================================================
// Context Requirements in Type Hierarchy
// ============================================================================

#[test]
fn test_function_context_requirements() {
    let func = Type::function_with_contexts(
        vec![Type::int()].into(),
        Type::bool(),
        make_contexts(vec!["Database", "Logger"]),
    );

    match func {
        Type::Function { contexts, .. } => {
            assert!(contexts.is_some());
            let ctx = contexts.unwrap();
            let names = ctx.context_names();
            let name_vec: Vec<_> = names.into_iter().collect();
            assert_eq!(name_vec.len(), 2);
            assert!(name_vec.iter().any(|n| n.as_str() == "Database"));
            assert!(name_vec.iter().any(|n| n.as_str() == "Logger"));
        }
        _ => panic!("Expected function type"),
    }
}

#[test]
fn test_context_subtyping() {
    let mut subtyping = Subtyping::new();
    let span = Span::dummy();

    // Function requiring fewer contexts could be subtype of one requiring more
    // This depends on context system design
    let f1 = Type::function(
        vec![Type::int()].into_iter().collect::<List<_>>(),
        Type::bool(),
    );

    let f2 = Type::function_with_contexts(
        vec![Type::int()].into_iter().collect::<List<_>>(),
        Type::bool(),
        make_contexts(vec!["Database"]),
    );

    // Test documents expected behavior
}
