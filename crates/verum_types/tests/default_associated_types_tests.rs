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
//! Tests for default associated type semantics in protocols
//!
//! Spec: grammar/verum.ebnf lines 416-417, 932-939
//!
//! Default associated types allow protocols to provide default type implementations
//! that are used when an implementation doesn't explicitly specify them.

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_common::{List, Map, Maybe};
use verum_types::protocol::{
    AssociatedType, Protocol, ProtocolBound, ProtocolChecker, ProtocolImpl, ProtocolMethod,
};
use verum_types::ty::Type;

/// Test that default associated types are correctly resolved
#[test]
fn test_default_associated_type_used() {
    let mut checker = ProtocolChecker::new();

    // Define Container protocol with default Item type
    let container_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Container".into(),
        type_params: List::new(),
        methods: {
            let mut methods = Map::new();
            methods.insert(
                "get".into(),
                ProtocolMethod {
                    name: "get".into(),
                    ty: Type::function(
                        List::from(vec![
                            Type::Named {
                                path: Path::single(Ident::new("Self", Span::default())),
                                args: List::new(),
                            },
                            Type::int(),
                        ]),
                        Type::Named {
                            path: Path::new(
                                vec![
                                    PathSegment::Name(Ident::new("Self", Span::default())),
                                    PathSegment::Name(Ident::new("Item", Span::default())),
                                ].into(),
                                Span::default(),
                            ),
                            args: List::new(),
                        },
                    ),
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
        associated_types: {
            let mut assoc = Map::new();
            assoc.insert(
                "Item".into(),
                AssociatedType {
                    name: "Item".into(),
                    type_params: List::new(),
                    bounds: List::new(),
                    where_clauses: List::new(),
                    default: Maybe::Some(Type::Named {
                        path: Path::new(
                            vec![PathSegment::Name(Ident::new("Heap", Span::default()))].into(),
                            Span::default(),
                        ),
                        args: List::from(vec![Type::Named {
                            path: Path::single(Ident::new("u8", Span::default())),
                            args: List::new(),
                        }]),
                    }),
                    kind: verum_types::advanced_protocols::AssociatedTypeKind::Regular,
                    refinement: Maybe::None,
                    expected_variance: verum_types::advanced_protocols::Variance::Invariant,
                },
            );
            assoc
        },
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("test".into()),
        span: Span::default(),
    };

    checker.register_protocol(container_protocol);

    // Implement Container for MyType WITHOUT specifying Item
    let my_type = Type::Named {
        path: Path::single(Ident::new("MyType", Span::default())),
        args: List::new(),
    };

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Container", Span::default())),
        protocol_args: List::new(),
        for_type: my_type.clone(),
        where_clauses: List::new(),
        methods: {
            let mut methods = Map::new();
            methods.insert(
                "get".into(),
                Type::function(
                    List::from(vec![my_type.clone(), Type::int()]),
                    Type::Named {
                        path: Path::new(
                            vec![PathSegment::Name(Ident::new("Heap", Span::default()))].into(),
                            Span::default(),
                        ),
                        args: List::from(vec![Type::Named {
                            path: Path::single(Ident::new("u8", Span::default())),
                            args: List::new(),
                        }]),
                    },
                ),
            );
            methods
        },
        associated_types: Map::new(), // NOT specified!
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::Some("test".into()),
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker
        .register_impl(impl_)
        .expect("Failed to register impl");

    // Resolve the associated type - should get the default Heap<u8>
    let result = checker.resolve_associated_type(
        &my_type,
        &Path::single(Ident::new("Container", Span::default())),
        &"Item".into(),
    );

    assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
    let resolved_ty = result.unwrap();

    // Should be Heap<u8>
    match &resolved_ty {
        Type::Named { path, args } => {
            assert_eq!(path.as_ident().unwrap().as_str(), "Heap");
            assert_eq!(args.len(), 1);
            match &args[0] {
                Type::Named { path, .. } => {
                    assert_eq!(path.as_ident().unwrap().as_str(), "u8");
                }
                _ => panic!("Expected u8 type, got: {:?}", args[0]),
            }
        }
        _ => panic!("Expected Named type, got: {:?}", resolved_ty),
    }
}

/// Test that explicit associated type overrides default
#[test]
fn test_explicit_associated_type_overrides_default() {
    let mut checker = ProtocolChecker::new();

    // Define Container protocol with default Item type
    let container_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Container".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: {
            let mut assoc = Map::new();
            assoc.insert(
                "Item".into(),
                AssociatedType {
                    name: "Item".into(),
                    type_params: List::new(),
                    bounds: List::new(),
                    where_clauses: List::new(),
                    default: Maybe::Some(Type::Named {
                        path: Path::new(
                            vec![PathSegment::Name(Ident::new("Heap", Span::default()))].into(),
                            Span::default(),
                        ),
                        args: List::from(vec![Type::Named {
                            path: Path::single(Ident::new("u8", Span::default())),
                            args: List::new(),
                        }]),
                    }),
                    kind: verum_types::advanced_protocols::AssociatedTypeKind::Regular,
                    refinement: Maybe::None,
                    expected_variance: verum_types::advanced_protocols::Variance::Invariant,
                },
            );
            assoc
        },
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("test".into()),
        span: Span::default(),
    };

    checker.register_protocol(container_protocol);

    // Implement Container for MyType WITH explicit Item = Int
    let my_type = Type::Named {
        path: Path::single(Ident::new("MyType", Span::default())),
        args: List::new(),
    };

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Container", Span::default())),
        protocol_args: List::new(),
        for_type: my_type.clone(),
        where_clauses: List::new(),
        methods: Map::new(),
        associated_types: {
            let mut assoc = Map::new();
            assoc.insert("Item".into(), Type::int());
            assoc
        },
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::Some("test".into()),
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker
        .register_impl(impl_)
        .expect("Failed to register impl");

    // Resolve the associated type - should get Int, not the default
    let result = checker.resolve_associated_type(
        &my_type,
        &Path::single(Ident::new("Container", Span::default())),
        &"Item".into(),
    );

    assert!(result.is_ok());
    let resolved_ty = result.unwrap();

    // Should be Int, not Heap<u8>
    assert!(
        matches!(resolved_ty, Type::Int),
        "Expected Int, got: {:?}",
        resolved_ty
    );
}

/// Test that missing associated type with no default produces error
#[test]
fn test_missing_associated_type_no_default_error() {
    let mut checker = ProtocolChecker::new();

    // Define Container protocol WITHOUT default Item type
    let container_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Container".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: {
            let mut assoc = Map::new();
            assoc.insert(
                "Item".into(),
                AssociatedType {
                    name: "Item".into(),
                    type_params: List::new(),
                    bounds: List::new(),
                    where_clauses: List::new(),
                    default: Maybe::None, // NO DEFAULT
                    kind: verum_types::advanced_protocols::AssociatedTypeKind::Regular,
                    refinement: Maybe::None,
                    expected_variance: verum_types::advanced_protocols::Variance::Invariant,
                },
            );
            assoc
        },
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("test".into()),
        span: Span::default(),
    };

    checker.register_protocol(container_protocol);

    // Implement Container for MyType WITHOUT specifying Item
    let my_type = Type::Named {
        path: Path::single(Ident::new("MyType", Span::default())),
        args: List::new(),
    };

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Container", Span::default())),
        protocol_args: List::new(),
        for_type: my_type.clone(),
        where_clauses: List::new(),
        methods: Map::new(),
        associated_types: Map::new(), // NOT specified!
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::Some("test".into()),
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker
        .register_impl(impl_)
        .expect("Failed to register impl");

    // Try to resolve the associated type - should error
    let result = checker.resolve_associated_type(
        &my_type,
        &Path::single(Ident::new("Container", Span::default())),
        &"Item".into(),
    );

    assert!(
        result.is_err(),
        "Expected error for missing associated type without default"
    );

    match result {
        Err(verum_types::protocol::ProtocolError::AssociatedTypeNotSpecified { .. }) => {
            // Expected error
        }
        _ => panic!(
            "Expected AssociatedTypeNotSpecified error, got: {:?}",
            result
        ),
    }
}

/// Test that default type must satisfy bounds
#[test]
fn test_default_type_validates_bounds() {
    // Use new_empty() for a clean checker without standard protocol registrations
    // This avoids conflicts when registering Clone for Int in the test
    let mut checker = ProtocolChecker::new_empty();

    // Register Clone protocol (simplified)
    let clone_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Clone".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("stdlib".into()),
        span: Span::default(),
    };
    checker.register_protocol(clone_protocol);

    // Implement Clone for Int
    let int_type = Type::int();
    let clone_impl = ProtocolImpl {
        protocol: Path::single(Ident::new("Clone", Span::default())),
        protocol_args: List::new(),
        for_type: int_type.clone(),
        where_clauses: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::Some("stdlib".into()),
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };
    checker
        .register_impl(clone_impl)
        .expect("Failed to register Clone for Int");

    // Define Container protocol with bounded default Item type
    let container_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Container".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: {
            let mut assoc = Map::new();
            assoc.insert(
                "Item".into(),
                AssociatedType {
                    name: "Item".into(),
                    type_params: List::new(),
                    bounds: List::from(vec![ProtocolBound {
                        protocol: Path::single(Ident::new("Clone", Span::default())),
                        args: List::new(),
                        is_negative: false,
                    }]),
                    where_clauses: List::new(),
                    default: Maybe::Some(Type::int()), // Int implements Clone
                    kind: verum_types::advanced_protocols::AssociatedTypeKind::Regular,
                    refinement: Maybe::None,
                    expected_variance: verum_types::advanced_protocols::Variance::Invariant,
                },
            );
            assoc
        },
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("test".into()),
        span: Span::default(),
    };

    checker.register_protocol(container_protocol);

    // Implement Container for MyType WITHOUT specifying Item
    let my_type = Type::Named {
        path: Path::single(Ident::new("MyType", Span::default())),
        args: List::new(),
    };

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Container", Span::default())),
        protocol_args: List::new(),
        for_type: my_type.clone(),
        where_clauses: List::new(),
        methods: Map::new(),
        associated_types: Map::new(), // NOT specified, will use default
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::Some("test".into()),
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker
        .register_impl(impl_)
        .expect("Failed to register impl");

    // Resolve the associated type - should succeed because Int implements Clone
    let result = checker.resolve_associated_type(
        &my_type,
        &Path::single(Ident::new("Container", Span::default())),
        &"Item".into(),
    );

    assert!(
        result.is_ok(),
        "Expected Ok because default type satisfies bounds, got: {:?}",
        result
    );
    let resolved_ty = result.unwrap();
    assert!(matches!(resolved_ty, Type::Int));
}
