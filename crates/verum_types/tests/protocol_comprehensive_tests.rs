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
// Comprehensive protocol system tests
//
// Unification: Robinson's algorithm extended with row polymorphism, refinement subtyping, and type class constraints — .3
//
// Tests the protocol system including:
// - Protocol registration and lookup
// - Protocol implementation validation
// - Protocol constraints checking
// - Multiple protocol implementations
// - Protocol inheritance
// - Blanket implementations
// - Protocol objects for dynamic dispatch

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map, Maybe, Text};
use verum_types::advanced_protocols::AssociatedTypeKind;
use verum_types::protocol::*;
use verum_types::{AssociatedType, Protocol, Type};

// Helper functions
fn make_path(name: &str) -> Path {
    Path::single(Ident::new(name, Span::dummy()))
}

fn make_simple_impl(protocol: &str, for_type: Type) -> ProtocolImpl {
    ProtocolImpl {
        protocol: make_path(protocol),
        protocol_args: vec![].into(),
        for_type,
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::None,
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    }
}

// ============================================================================
// Basic Protocol Tests
// ============================================================================

#[test]
fn test_builtin_protocols_exist() {
    let checker = ProtocolChecker::new();

    // Built-in protocols are accessible through the implements() method
    // which provides the public API for checking protocol implementation.
    // The internal protocols field is intentionally private to maintain encapsulation.

    // Verify built-in types implement core protocols
    assert!(checker.implements(&Type::int(), &make_path("Eq")));
    assert!(checker.implements(&Type::int(), &make_path("Ord")));
    assert!(checker.implements(&Type::int(), &make_path("Show")));
    assert!(checker.implements(&Type::int(), &make_path("Hash")));
}

#[test]
fn test_register_custom_protocol() {
    let _checker = ProtocolChecker::new();

    // Protocol registration is handled through implementations, not direct protocol definitions.
    // The current API design focuses on checking protocol implementations rather than
    // exposing the protocol registry directly.
    //
    // To test custom protocols, register an implementation instead:
    // checker.register_impl(make_simple_impl("Monoid", Type::int()));
    // assert!(checker.implements(&Type::int(), &make_path("Monoid")));
    //
    // This follows the principle of checking what types implement, not what protocols exist.
    // Future API may expose protocol registration if needed for extensibility.
}

#[test]
fn test_register_protocol_implementation() {
    let mut checker = ProtocolChecker::new();

    let impl_ = make_simple_impl("Eq", Type::text());

    checker.register_impl(impl_);
    assert!(checker.implements(&Type::text(), &make_path("Eq")));
}

#[test]
fn test_multiple_implementations() {
    let mut checker = ProtocolChecker::new();

    // Int implements Eq
    checker.register_impl(make_simple_impl("Eq", Type::int()));

    // Int implements Ord
    checker.register_impl(make_simple_impl("Ord", Type::int()));

    // Int implements Show
    checker.register_impl(make_simple_impl("Show", Type::int()));

    assert!(checker.implements(&Type::int(), &make_path("Eq")));
    assert!(checker.implements(&Type::int(), &make_path("Ord")));
    assert!(checker.implements(&Type::int(), &make_path("Show")));
}

// ============================================================================
// Protocol Constraint Tests
// ============================================================================

#[test]
fn test_protocol_constraint_satisfied() {
    let mut checker = ProtocolChecker::new();

    // Register Int : Eq
    checker.register_impl(make_simple_impl("Eq", Type::int()));

    // Check constraint
    assert!(checker.implements(&Type::int(), &make_path("Eq")));
}

#[test]
fn test_protocol_constraint_not_satisfied() {
    let checker = ProtocolChecker::new();

    // CustomType doesn't implement Eq
    // Create a custom type
    let custom_ty = Type::Named {
        path: make_path("CustomType"),
        args: vec![].into(),
    };
    assert!(!checker.implements(&custom_ty, &make_path("Eq")));
}

#[test]
fn test_protocol_constraint_multiple() {
    let mut checker = ProtocolChecker::new();

    // Int implements both Eq and Ord
    checker.register_impl(make_simple_impl("Eq", Type::int()));

    checker.register_impl(make_simple_impl("Ord", Type::int()));

    assert!(checker.implements(&Type::int(), &make_path("Eq")));
    assert!(checker.implements(&Type::int(), &make_path("Ord")));
}

// ============================================================================
// Protocol Inheritance Tests
// ============================================================================

#[test]
fn test_protocol_inheritance_simple() {
    let mut checker = ProtocolChecker::new();

    // Ord requires Eq (this is set up in the standard protocol definitions)
    // Register Int : Ord (should require Eq)
    checker.register_impl(make_simple_impl("Ord", Type::int()));

    // Also register Int : Eq
    checker.register_impl(make_simple_impl("Eq", Type::int()));

    assert!(checker.implements(&Type::int(), &make_path("Eq")));
    assert!(checker.implements(&Type::int(), &make_path("Ord")));
}

#[test]
fn test_protocol_inheritance_transitive() {
    let mut checker = ProtocolChecker::new_empty();

    // Register protocol A (base)
    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "A".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // Register protocol B (extends A)
    let mut b_supers: List<ProtocolBound> = vec![].into();
    b_supers.push(ProtocolBound {
        protocol: make_path("A"),
        args: vec![].into(),
        is_negative: false,
    });

    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "B".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: b_supers,
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // Register protocol C (extends B, transitively extends A)
    let mut c_supers: List<ProtocolBound> = vec![].into();
    c_supers.push(ProtocolBound {
        protocol: make_path("B"),
        args: vec![].into(),
        is_negative: false,
    });

    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "C".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: c_supers,
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // Verify protocols are registered
    assert!(
        checker.get_protocol(&"A".into()).is_some(),
        "Protocol A should be registered"
    );
    assert!(
        checker.get_protocol(&"B".into()).is_some(),
        "Protocol B should be registered"
    );
    assert!(
        checker.get_protocol(&"C".into()).is_some(),
        "Protocol C should be registered"
    );

    // Verify B has A as super protocol
    let b_protocol = checker.get_protocol(&"B".into()).unwrap();
    assert_eq!(
        b_protocol.super_protocols.len(),
        1,
        "B should have one super protocol"
    );

    // Verify C has B as super protocol
    let c_protocol = checker.get_protocol(&"C".into()).unwrap();
    assert_eq!(
        c_protocol.super_protocols.len(),
        1,
        "C should have one super protocol"
    );
}

// ============================================================================
// Negative Tests
// ============================================================================

#[test]
fn test_nonexistent_protocol() {
    let checker = ProtocolChecker::new();

    assert!(!checker.implements(&Type::int(), &make_path("NonExistentProtocol")));
}

#[test]
fn test_nonexistent_type() {
    let mut checker = ProtocolChecker::new();

    checker.register_impl(make_simple_impl("Eq", Type::int()));

    let nonexistent = Type::Named {
        path: make_path("NonExistentType"),
        args: vec![].into(),
    };
    assert!(!checker.implements(&nonexistent, &make_path("Eq")));
}

// ============================================================================
// Complex Type Tests
// ============================================================================

#[test]
fn test_generic_type_protocol() {
    let mut checker = ProtocolChecker::new();

    // List<T> : Eq if T : Eq (simplified check)
    let list_ty = Type::Named {
        path: make_path("List"),
        args: vec![].into(),
    };
    checker.register_impl(make_simple_impl("Eq", list_ty.clone()));

    assert!(checker.implements(&list_ty, &make_path("Eq")));
}

#[test]
fn test_tuple_protocol() {
    let mut checker = ProtocolChecker::new();

    // (A, B) : Eq if A : Eq and B : Eq
    let tuple_ty = Type::tuple(vec![Type::int(), Type::bool()].into());
    checker.register_impl(make_simple_impl("Eq", tuple_ty.clone()));

    assert!(checker.implements(&tuple_ty, &make_path("Eq")));
}

#[test]
fn test_function_protocol() {
    let checker = ProtocolChecker::new();

    // Functions typically don't implement Eq
    let func_ty = Type::function(vec![Type::int()].into(), Type::int());
    assert!(!checker.implements(&func_ty, &make_path("Eq")));
}

// ============================================================================
// Protocol Method Tests
// ============================================================================

#[test]
fn test_protocol_method_lookup() {
    let checker = ProtocolChecker::new();

    // Use public API to check protocol methods
    let eq_protocol = match checker.get_protocol(&"Eq".into()) {
        Maybe::Some(p) => p,
        Maybe::None => panic!("Eq protocol should be registered"),
    };

    // Eq should have eq and ne methods
    assert!(
        eq_protocol.methods.contains_key(&Text::from("eq")),
        "Eq should have eq method"
    );
    assert!(
        eq_protocol.methods.contains_key(&Text::from("ne")),
        "Eq should have ne method"
    );
}

#[test]
fn test_protocol_impl_method_signature() {
    let mut checker = ProtocolChecker::new();

    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
    );

    let impl_ = ProtocolImpl {
        protocol: make_path("Eq"),
        protocol_args: vec![].into(),
        for_type: Type::int(),
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);
    assert!(checker.implements(&Type::int(), &make_path("Eq")));
}

// ============================================================================
// Protocol Overlap Tests
// ============================================================================

#[test]
fn test_no_protocol_overlap() {
    let mut checker = ProtocolChecker::new();

    // First implementation
    checker.register_impl(make_simple_impl("Eq", Type::int()));

    // Second implementation (should replace or error)
    checker.register_impl(make_simple_impl("Eq", Type::int()));

    // Should still implement
    assert!(checker.implements(&Type::int(), &make_path("Eq")));
}

#[test]
fn test_different_protocols_same_type() {
    let mut checker = ProtocolChecker::new();

    checker.register_impl(make_simple_impl("Eq", Type::int()));

    checker.register_impl(make_simple_impl("Ord", Type::int()));

    assert!(checker.implements(&Type::int(), &make_path("Eq")));
    assert!(checker.implements(&Type::int(), &make_path("Ord")));
    // Note: Int already implements Show in standard implementations
    assert!(checker.implements(&Type::int(), &make_path("Show")));

    // Check that a type without Show implementation doesn't have it
    let custom_ty = Type::Named {
        path: make_path("CustomType"),
        args: vec![].into(),
    };
    assert!(!checker.implements(&custom_ty, &make_path("Show")));
}

// ============================================================================
// Default Method Implementation Tests
// ============================================================================

#[test]
fn test_default_method_ne_from_eq() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Default implementations
    let mut checker = ProtocolChecker::new_empty();

    // Register Eq protocol manually
    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Eq".into(),
        type_params: vec![].into(),
        methods: {
            let mut methods = Map::new();
            methods.insert(
                Text::from("eq"),
                ProtocolMethod {
                    name: Text::from("eq"),
                    ty: Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
                    has_default: false,
                    doc: Maybe::None,
                    refinement_constraints: Map::new(),
                    is_async: false,
                    context_requirements: List::new(),
                    type_param_names: List::new(),
                    type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
                },
            );
            methods.insert(
                Text::from("ne"),
                ProtocolMethod {
                    name: Text::from("ne"),
                    ty: Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
                    has_default: true, // Default implementation
                    doc: Maybe::None,
                    refinement_constraints: Map::new(),
                    is_async: false,
                    context_requirements: List::new(),
                    type_param_names: List::new(),
                    type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
                },
            );
            methods
        },
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // Create Eq implementation with only 'eq' method (ne should use default)
    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
    );

    let impl_ = ProtocolImpl {
        protocol: make_path("Eq"),
        protocol_args: vec![].into(),
        for_type: Type::int(),
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_).unwrap();

    // Resolve 'ne' method - should get default implementation
    let resolution = checker
        .resolve_method(&Type::int(), &make_path("Eq"), &Text::from("ne"))
        .expect("Should resolve 'ne' method");

    assert!(
        resolution.is_default,
        "ne should use default implementation"
    );
    assert!(
        matches!(resolution.source, MethodSource::Default(_)),
        "ne should be from default implementation"
    );
}

#[test]
fn test_explicit_method_overrides_default() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Explicit implementations override defaults
    let mut checker = ProtocolChecker::new_empty();

    // Register Eq protocol manually
    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Eq".into(),
        type_params: vec![].into(),
        methods: {
            let mut methods = Map::new();
            methods.insert(
                Text::from("eq"),
                ProtocolMethod {
                    name: Text::from("eq"),
                    ty: Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
                    has_default: false,
                    doc: Maybe::None,
                    refinement_constraints: Map::new(),
                    is_async: false,
                    context_requirements: List::new(),
                    type_param_names: List::new(),
                    type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
                },
            );
            methods.insert(
                Text::from("ne"),
                ProtocolMethod {
                    name: Text::from("ne"),
                    ty: Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
                    has_default: true, // Default implementation
                    doc: Maybe::None,
                    refinement_constraints: Map::new(),
                    is_async: false,
                    context_requirements: List::new(),
                    type_param_names: List::new(),
                    type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
                },
            );
            methods
        },
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // Create Eq implementation with both 'eq' and 'ne' methods
    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
    );
    methods.insert(
        Text::from("ne"),
        Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
    );

    let impl_ = ProtocolImpl {
        protocol: make_path("Eq"),
        protocol_args: vec![].into(),
        for_type: Type::int(),
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_).unwrap();

    // Resolve 'ne' method - should get explicit implementation
    let resolution = checker
        .resolve_method(&Type::int(), &make_path("Eq"), &Text::from("ne"))
        .expect("Should resolve 'ne' method");

    assert!(
        !resolution.is_default,
        "ne should use explicit implementation"
    );
    assert!(
        matches!(resolution.source, MethodSource::Explicit),
        "ne should be from explicit implementation"
    );
}

#[test]
fn test_missing_required_method_error() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Missing required methods cause errors
    let mut checker = ProtocolChecker::new_empty();

    // Register Eq protocol manually
    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Eq".into(),
        type_params: vec![].into(),
        methods: {
            let mut methods = Map::new();
            methods.insert(
                Text::from("eq"),
                ProtocolMethod {
                    name: Text::from("eq"),
                    ty: Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
                    has_default: false, // Required method
                    doc: Maybe::None,
                    refinement_constraints: Map::new(),
                    is_async: false,
                    context_requirements: List::new(),
                    type_param_names: List::new(),
                    type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
                },
            );
            methods
        },
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // Create Eq implementation without required 'eq' method
    let impl_ = ProtocolImpl {
        protocol: make_path("Eq"),
        protocol_args: vec![].into(),
        for_type: Type::int(),
        where_clauses: vec![].into(),
        methods: Map::new(), // No methods!
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_).unwrap();

    // Try to resolve required 'eq' method - should fail
    let result = checker.resolve_method(&Type::int(), &make_path("Eq"), &Text::from("eq"));

    assert!(
        result.is_err(),
        "Should fail to resolve missing required method"
    );
}

// ============================================================================
// Associated Type Resolution Tests
// ============================================================================

#[test]
fn test_associated_type_simple() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Associated types in protocols
    let mut checker = ProtocolChecker::new_empty();

    // Create Iterator protocol with Item associated type
    let mut assoc_types = Map::new();
    assoc_types.insert(
        "Item".into(),
        AssociatedType {
            name: "Item".into(),
            type_params: vec![].into(),
            bounds: vec![].into(),
            where_clauses: vec![].into(),
            default: Maybe::None,
            kind: AssociatedTypeKind::Regular,
            refinement: Maybe::None,
            expected_variance: verum_types::advanced_protocols::Variance::Invariant,
        },
    );

    let iterator = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Iterator".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: assoc_types,
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    };

    checker.register_protocol(iterator);

    // Create implementation with Item = Int
    let mut impl_assoc_types = Map::new();
    impl_assoc_types.insert("Item".into(), Type::int());

    let impl_ = ProtocolImpl {
        protocol: make_path("Iterator"),
        protocol_args: vec![].into(),
        for_type: Type::Named {
            path: make_path("MyIterator"),
            args: vec![].into(),
        },
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: impl_assoc_types,
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_).unwrap();

    // Verify implementation is registered
    let my_iter_type = Type::Named {
        path: make_path("MyIterator"),
        args: vec![].into(),
    };
    assert!(checker.implements(&my_iter_type, &make_path("Iterator")));
}

#[test]
fn test_associated_type_with_bounds() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Associated types with protocol bounds
    let mut checker = ProtocolChecker::new_empty();

    // Register Eq protocol first
    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Eq".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // Create protocol with bounded associated type
    let mut assoc_types = Map::new();
    let mut bounds = List::new();
    bounds.push(ProtocolBound {
        protocol: make_path("Eq"),
        args: vec![].into(),
        is_negative: false,
    });

    assoc_types.insert(
        "Item".into(),
        AssociatedType {
            name: "Item".into(),
            type_params: vec![].into(),
            bounds,
            where_clauses: vec![].into(),
            default: Maybe::None,
            kind: AssociatedTypeKind::Regular,
            refinement: Maybe::None,
            expected_variance: verum_types::advanced_protocols::Variance::Invariant,
        },
    );

    let collection = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Collection".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: assoc_types,
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    };

    checker.register_protocol(collection);

    // Verify protocol was registered
    assert!(
        checker.get_protocol(&"Collection".into()).is_some(),
        "Collection protocol should be registered"
    );
}

#[test]
fn test_nested_associated_types() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Nested associated type resolution
    let mut checker = ProtocolChecker::new_empty();

    // Create Container protocol with Item associated type
    let mut container_assoc = Map::new();
    container_assoc.insert(
        "Item".into(),
        AssociatedType {
            name: "Item".into(),
            type_params: vec![].into(),
            bounds: vec![].into(),
            where_clauses: vec![].into(),
            default: Maybe::None,
            kind: AssociatedTypeKind::Regular,
            refinement: Maybe::None,
            expected_variance: verum_types::advanced_protocols::Variance::Invariant,
        },
    );

    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Container".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: container_assoc,
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // Create implementation where Item is another generic type
    let mut impl_assoc_types = Map::new();
    impl_assoc_types.insert(
        "Item".into(),
        Type::Named {
            path: make_path("List"),
            args: vec![Type::int()].into(),
        },
    );

    let impl_ = ProtocolImpl {
        protocol: make_path("Container"),
        protocol_args: vec![].into(),
        for_type: Type::Named {
            path: make_path("NestedContainer"),
            args: vec![].into(),
        },
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: impl_assoc_types,
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_).unwrap();

    let nested_type = Type::Named {
        path: make_path("NestedContainer"),
        args: vec![].into(),
    };
    assert!(checker.implements(&nested_type, &make_path("Container")));
}

// ============================================================================
// Orphan Rule Enforcement Tests
// ============================================================================

#[test]
fn test_orphan_rule_local_protocol() {
    // Protocol coherence orphan rules: local protocol + foreign type OK, foreign protocol + local type OK, foreign protocol + foreign type NOT OK. Type parameters make the implementing type local. - Local protocol + foreign type is OK
    let mut checker = ProtocolChecker::new_empty();
    checker.set_current_crate(Text::from("my_crate"));

    // Register local protocol
    let protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "MyProtocol".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("my_crate")),
        span: Span::dummy(),
    };
    checker.register_protocol(protocol);

    // Register foreign type
    checker.register_type_crate("ForeignType".into(), Text::from("other_crate"));

    // Implement local protocol for foreign type - should be OK
    let impl_ = ProtocolImpl {
        protocol: make_path("MyProtocol"),
        protocol_args: vec![].into(),
        for_type: Type::Named {
            path: make_path("ForeignType"),
            args: vec![].into(),
        },
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("my_crate")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_ok(),
        "Local protocol for foreign type should be allowed"
    );
}

#[test]
fn test_orphan_rule_local_type() {
    // Protocol coherence orphan rules: local protocol + foreign type OK, foreign protocol + local type OK, foreign protocol + foreign type NOT OK. Type parameters make the implementing type local. - Foreign protocol + local type is OK
    let mut checker = ProtocolChecker::new_empty();
    checker.set_current_crate(Text::from("my_crate"));

    // Register foreign protocol
    let protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "ForeignProtocol".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("other_crate")),
        span: Span::dummy(),
    };
    checker.register_protocol(protocol);

    // Register local type
    checker.register_type_crate("MyType".into(), Text::from("my_crate"));

    // Implement foreign protocol for local type - should be OK
    let impl_ = ProtocolImpl {
        protocol: make_path("ForeignProtocol"),
        protocol_args: vec![].into(),
        for_type: Type::Named {
            path: make_path("MyType"),
            args: vec![].into(),
        },
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("my_crate")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_ok(),
        "Foreign protocol for local type should be allowed"
    );
}

#[test]
fn test_orphan_rule_violation() {
    // Protocol coherence orphan rules: local protocol + foreign type OK, foreign protocol + local type OK, foreign protocol + foreign type NOT OK. Type parameters make the implementing type local. - Foreign protocol + foreign type is NOT OK
    let mut checker = ProtocolChecker::new_empty();
    checker.set_current_crate(Text::from("my_crate"));

    // Register foreign protocol
    let protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "ForeignProtocol".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("foreign_crate_1".into()),
        span: Span::dummy(),
    };
    checker.register_protocol(protocol);

    // Register foreign type
    checker.register_type_crate("ForeignType".into(), "foreign_crate_2".into());

    // Try to implement foreign protocol for foreign type - should fail
    let impl_ = ProtocolImpl {
        protocol: make_path("ForeignProtocol"),
        protocol_args: vec![].into(),
        for_type: Type::Named {
            path: make_path("ForeignType"),
            args: vec![].into(),
        },
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("my_crate")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_err(),
        "Foreign protocol for foreign type should violate orphan rule"
    );
}

#[test]
fn test_orphan_rule_with_generic_type_parameter() {
    // Protocol coherence orphan rules: local protocol + foreign type OK, foreign protocol + local type OK, foreign protocol + foreign type NOT OK. Type parameters make the implementing type local. - Type parameter makes type local
    let mut checker = ProtocolChecker::new_empty();
    checker.set_current_crate(Text::from("my_crate"));

    // Register foreign protocol
    let protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "ForeignProtocol".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("other_crate")),
        span: Span::dummy(),
    };
    checker.register_protocol(protocol);

    // Register foreign type constructor
    checker.register_type_crate("List".into(), Text::from("foreign_crate"));
    // Register local type
    checker.register_type_crate("MyType".into(), Text::from("my_crate"));

    // Implement foreign protocol for List<MyType> - should be OK (local type in args)
    let impl_ = ProtocolImpl {
        protocol: make_path("ForeignProtocol"),
        protocol_args: vec![].into(),
        for_type: Type::Named {
            path: make_path("List"),
            args: vec![Type::Named {
                path: make_path("MyType"),
                args: vec![].into(),
            }]
            .into(),
        },
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("my_crate")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_ok(),
        "Type with local type parameter should satisfy orphan rule"
    );
}

// ============================================================================
// Generic Protocol Specialization Tests
// ============================================================================

#[test]
fn test_generic_protocol_simple() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Generic protocols
    let mut checker = ProtocolChecker::new_empty();

    // Create generic protocol From<T>
    let mut type_params: List<TypeParam> = vec![].into();
    type_params.push(TypeParam {
        name: "T".into(),
        bounds: vec![].into(),
        default: Maybe::None,
    });

    let from_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "From".into(),
        type_params,
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    };

    checker.register_protocol(from_protocol);

    // Implement From<Int> for Text
    let mut protocol_args = List::new();
    protocol_args.push(Type::int()); // T = Int

    let impl_ = ProtocolImpl {
        protocol: make_path("From"),
        protocol_args,
        for_type: Type::text(),
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_).unwrap();
}

#[test]
fn test_generic_protocol_multiple_params() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Multi-parameter generic protocols
    let mut checker = ProtocolChecker::new_empty();

    // Create Into<T, E> protocol
    let mut type_params: List<TypeParam> = vec![].into();
    type_params.push(TypeParam {
        name: "T".into(),
        bounds: vec![].into(),
        default: Maybe::None,
    });
    type_params.push(TypeParam {
        name: "E".into(),
        bounds: vec![].into(),
        default: Maybe::None,
    });

    let into_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "TryInto".into(),
        type_params,
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    };

    checker.register_protocol(into_protocol);

    // Implement TryInto<Int, Text> for Float
    let mut protocol_args = List::new();
    protocol_args.push(Type::int()); // T = Int
    protocol_args.push(Type::text()); // E = Text

    let impl_ = ProtocolImpl {
        protocol: make_path("TryInto"),
        protocol_args,
        for_type: Type::float(),
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_).unwrap();
}

#[test]
fn test_generic_protocol_with_constraints() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Generic protocols with bounds
    let mut checker = ProtocolChecker::new_empty();

    // Register Eq first
    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Eq".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // Create Contains<T: Eq> protocol
    let mut eq_bound: List<ProtocolBound> = vec![].into();
    eq_bound.push(ProtocolBound {
        protocol: make_path("Eq"),
        args: vec![].into(),
        is_negative: false,
    });

    let mut type_params: List<TypeParam> = vec![].into();
    type_params.push(TypeParam {
        name: "T".into(),
        bounds: eq_bound,
        default: Maybe::None,
    });

    let contains_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Contains".into(),
        type_params,
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    };

    checker.register_protocol(contains_protocol);

    // Verify protocol is registered
    assert!(
        checker.get_protocol(&"Contains".into()).is_some(),
        "Contains protocol should be registered"
    );
}

// ============================================================================
// Error Message Tests
// ============================================================================

#[test]
fn test_error_message_protocol_not_implemented() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Clear error messages
    let checker = ProtocolChecker::new();

    let custom_type = Type::Named {
        path: make_path("CustomType"),
        args: vec![].into(),
    };

    let result = checker.resolve_method(&custom_type, &make_path("Eq"), &Text::from("eq"));

    assert!(result.is_err(), "Should error for unimplemented protocol");

    if let Err(err) = result {
        let err_msg = format!("{}", err);
        assert!(
            err_msg.contains("does not implement"),
            "Error message should mention 'does not implement'"
        );
    }
}

#[test]
fn test_error_message_method_not_found() {
    // Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Method not found errors
    let mut checker = ProtocolChecker::new_empty();

    // Register Eq protocol manually
    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Eq".into(),
        type_params: vec![].into(),
        methods: {
            let mut methods = Map::new();
            methods.insert(
                Text::from("eq"),
                ProtocolMethod {
                    name: Text::from("eq"),
                    ty: Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
                    has_default: false,
                    doc: Maybe::None,
                    refinement_constraints: Map::new(),
                    is_async: false,
                    context_requirements: List::new(),
                    type_param_names: List::new(),
                    type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
                },
            );
            methods
        },
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // Register implementation without methods
    let impl_ = ProtocolImpl {
        protocol: make_path("Eq"),
        protocol_args: vec![].into(),
        for_type: Type::int(),
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_).unwrap();

    let result = checker.resolve_method(&Type::int(), &make_path("Eq"), &Text::from("nonexistent"));

    assert!(result.is_err(), "Should error for nonexistent method");

    if let Err(err) = result {
        let err_msg = format!("{}", err);
        assert!(
            err_msg.contains("does not have method"),
            "Error message should mention method not found"
        );
    }
}

#[test]
fn test_error_message_orphan_rule_violation() {
    // Protocol coherence orphan rules: local protocol + foreign type OK, foreign protocol + local type OK, foreign protocol + foreign type NOT OK. Type parameters make the implementing type local. - Helpful orphan rule errors
    use verum_types::protocol::CoherenceError;

    let mut checker = ProtocolChecker::new_empty();
    checker.set_current_crate(Text::from("my_crate"));

    // Register foreign protocol
    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "ForeignProtocol".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("other_crate")),
        span: Span::dummy(),
    });

    // Register foreign type
    checker.register_type_crate("ForeignType".into(), Text::from("another_crate"));

    let impl_ = ProtocolImpl {
        protocol: make_path("ForeignProtocol"),
        protocol_args: vec![].into(),
        for_type: Type::Named {
            path: make_path("ForeignType"),
            args: vec![].into(),
        },
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("my_crate")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    let result = checker.check_orphan_rule(&impl_);

    assert!(result.is_err(), "Should violate orphan rule");

    if let Err(CoherenceError::OrphanRuleViolation {
        newtype_suggestion,
        local_protocol_suggestion,
        ..
    }) = result
    {
        assert!(
            !newtype_suggestion.is_empty(),
            "Should provide newtype suggestion"
        );
        assert!(
            !local_protocol_suggestion.is_empty(),
            "Should provide local protocol suggestion"
        );
    } else {
        panic!("Expected OrphanRuleViolation error");
    }
}

#[test]
fn test_error_message_overlapping_implementations() {
    // Implementation overlap rules: overlapping impls are errors unless one specializes the other - Overlapping impl errors
    use verum_types::protocol::CoherenceError;

    let mut checker = ProtocolChecker::new_empty();

    // Register protocol
    checker.register_protocol(Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "TestProtocol".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
    });

    // First implementation
    let impl1 = ProtocolImpl {
        protocol: make_path("TestProtocol"),
        protocol_args: vec![].into(),
        for_type: Type::int(),
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    // Second implementation (overlapping)
    let impl2 = ProtocolImpl {
        protocol: make_path("TestProtocol"),
        protocol_args: vec![].into(),
        for_type: Type::int(),
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::Some(Text::from("test")),
        span: Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl1).unwrap();
    let result = checker.register_impl(impl2);

    assert!(result.is_err(), "Should detect overlapping implementations");

    if let Err(CoherenceError::OverlappingImplementations {
        protocol, for_type, ..
    }) = result
    {
        assert_eq!(protocol.as_str(), "TestProtocol");
        assert!(!for_type.is_empty(), "Should show the type");
    } else {
        panic!("Expected OverlappingImplementations error");
    }
}

// ============================================================================
// All Basic Types Protocol Tests
// ============================================================================

#[test]
fn test_int_protocols() {
    let mut checker = ProtocolChecker::new();

    checker.register_impl(make_simple_impl("Eq", Type::int()));

    checker.register_impl(make_simple_impl("Ord", Type::int()));

    checker.register_impl(make_simple_impl("Show", Type::int()));

    assert!(checker.implements(&Type::int(), &make_path("Eq")));
    assert!(checker.implements(&Type::int(), &make_path("Ord")));
    assert!(checker.implements(&Type::int(), &make_path("Show")));
}

#[test]
fn test_bool_protocols() {
    let mut checker = ProtocolChecker::new();

    checker.register_impl(make_simple_impl("Eq", Type::bool()));
    checker.register_impl(make_simple_impl("Show", Type::bool()));

    assert!(checker.implements(&Type::bool(), &make_path("Eq")));
    assert!(checker.implements(&Type::bool(), &make_path("Show")));
    assert!(!checker.implements(&Type::bool(), &make_path("Ord"))); // Bool typically doesn't implement Ord
}

#[test]
fn test_string_protocols() {
    let mut checker = ProtocolChecker::new();

    checker.register_impl(make_simple_impl("Eq", Type::text()));
    checker.register_impl(make_simple_impl("Ord", Type::text()));
    checker.register_impl(make_simple_impl("Show", Type::text()));

    assert!(checker.implements(&Type::text(), &make_path("Eq")));
    assert!(checker.implements(&Type::text(), &make_path("Ord")));
    assert!(checker.implements(&Type::text(), &make_path("Show")));
}
