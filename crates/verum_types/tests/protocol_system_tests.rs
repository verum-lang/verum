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
// Comprehensive Protocol System Tests
//
// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Complete Protocol System
//
// Tests cover:
// - Protocol inheritance (superprotocols)
// - Method resolution with default implementations
// - VTable generation and dispatch
// - Associated types and constants
// - where type T: Protocol constraints

use verum_ast::{
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{List, Map, Maybe, Text};
use verum_types::{
    AssociatedType, MethodSource, Protocol, ProtocolBound, ProtocolChecker, ProtocolImpl,
    ProtocolMethod, Type,
};

// ==================== Helper Functions ====================

fn create_eq_protocol() -> Protocol {
    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        ProtocolMethod {
            name: Text::from("eq"),
            ty: Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
            has_default: false,
            doc: Maybe::Some("Test for equality".into()),
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
            ty: Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
            has_default: true, // Default implementation
            doc: Maybe::Some("Test for inequality (has default impl)".into()),
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        },
    );

    Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Eq".into(),
        type_params: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    }
}

fn create_ord_protocol() -> Protocol {
    let mut methods = Map::new();
    methods.insert(
        Text::from("cmp"),
        ProtocolMethod {
            name: Text::from("cmp"),
            ty: Type::function(vec![Type::Int, Type::Int].into(), Type::Int),
            has_default: false,
            doc: Maybe::Some("Compare two values".into()),
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        },
    );
    methods.insert(
        Text::from("lt"),
        ProtocolMethod {
            name: Text::from("lt"),
            ty: Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
            has_default: true, // Derived from cmp
            doc: Maybe::Some("Less than (has default impl)".into()),
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        },
    );
    methods.insert(
        Text::from("le"),
        ProtocolMethod {
            name: Text::from("le"),
            ty: Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
            has_default: true, // Derived from cmp
            doc: Maybe::Some("Less than or equal (has default impl)".into()),
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        },
    );

    let mut super_protocols = List::new();
    super_protocols.push(ProtocolBound {
        protocol: Path::single(Ident::new("Eq", Span::default())),
        args: List::new(),
        is_negative: false,
    });

    Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Ord".into(),
        type_params: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols, // Ord inherits Eq
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    }
}

fn create_iterator_protocol() -> Protocol {
    let mut methods = Map::new();
    methods.insert(
        Text::from("next"),
        ProtocolMethod {
            name: Text::from("next"),
            ty: Type::function(vec![Type::Int].into(), Type::Int), // Simplified
            has_default: false,
            doc: Maybe::Some("Get next element".into()),
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        },
    );
    methods.insert(
        Text::from("count"),
        ProtocolMethod {
            name: Text::from("count"),
            ty: Type::function(vec![Type::Int].into(), Type::Int),
            has_default: true, // Default impl using next()
            doc: Maybe::Some("Count elements (has default impl)".into()),
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        },
    );

    let mut associated_types = Map::new();
    associated_types.insert(
        "Item".into(),
        AssociatedType {
            name: "Item".into(),
            type_params: List::new(),
            bounds: List::new(),
            where_clauses: vec![].into(),
            default: Maybe::None,
            kind: verum_types::advanced_protocols::AssociatedTypeKind::Regular,
            refinement: Maybe::None,
            expected_variance: verum_types::advanced_protocols::Variance::Invariant,
        },
    );

    Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Iterator".into(),
        type_params: List::new(),
        methods,
        associated_types,
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    }
}

// ==================== Test: Protocol Registration ====================

#[test]
fn test_protocol_registration() {
    let checker = ProtocolChecker::new();

    // Standard protocols should be registered
    assert!(checker.get_protocol(&"Eq".into()).is_some());
    assert!(checker.get_protocol(&"Ord".into()).is_some());
    assert!(checker.get_protocol(&"Show".into()).is_some());
    assert!(checker.get_protocol(&"Iterator".into()).is_some());
}

#[test]
fn test_custom_protocol_registration() {
    let mut checker = ProtocolChecker::new();
    let custom_proto = create_eq_protocol();

    checker.register_protocol(custom_proto.clone());

    assert!(checker.get_protocol(&"Eq".into()).is_some());
}

// ==================== Test: Protocol Inheritance ====================

#[test]
fn test_protocol_inheritance_detection() {
    let mut checker = ProtocolChecker::new();

    // Register Eq first
    checker.register_protocol(create_eq_protocol());

    // Register Ord (inherits Eq)
    checker.register_protocol(create_ord_protocol());

    // Check inheritance
    assert!(checker.inherits_from(&"Ord".into(), &"Eq".into()));
    assert!(checker.inherits_from(&"Ord".into(), &"Ord".into())); // Self-inheritance
    assert!(!checker.inherits_from(&"Eq".into(), &"Ord".into())); // Not reverse
}

#[test]
fn test_transitive_inheritance() {
    let mut checker = ProtocolChecker::new();

    // Create protocol hierarchy: Show <- Debug <- DetailedDebug
    let show = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Show".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    let mut debug_supers = List::new();
    debug_supers.push(ProtocolBound {
        protocol: Path::single(Ident::new("Show", Span::default())),
        args: List::new(),
        is_negative: false,
    });

    let debug = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Debug".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: debug_supers,
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    let mut detailed_supers = List::new();
    detailed_supers.push(ProtocolBound {
        protocol: Path::single(Ident::new("Debug", Span::default())),
        args: List::new(),
        is_negative: false,
    });

    let detailed_debug = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "DetailedDebug".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: detailed_supers,
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    checker.register_protocol(show);
    checker.register_protocol(debug);
    checker.register_protocol(detailed_debug);

    // Test transitive inheritance
    assert!(checker.inherits_from(&"DetailedDebug".into(), &"Show".into()));
    assert!(checker.inherits_from(&"DetailedDebug".into(), &"Debug".into()));
    assert!(checker.inherits_from(&"Debug".into(), &"Show".into()));
}

// ==================== Test: Method Resolution ====================

#[test]
fn test_explicit_method_resolution() {
    let mut checker = ProtocolChecker::new_empty();
    checker.register_protocol(create_eq_protocol());

    // Create implementation for Int
    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Eq", Span::default())),
        protocol_args: vec![].into(),
        for_type: Type::Int,
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker
        .register_impl(impl_)
        .expect("Failed to register implementation");

    // Resolve method
    let resolution = checker
        .resolve_method(
            &Type::Int,
            &Path::single(Ident::new("Eq", Span::default())),
            &Text::from("eq"),
        )
        .expect("Method should resolve");

    assert!(!resolution.is_default);
    assert!(matches!(resolution.source, MethodSource::Explicit));
}

#[test]
fn test_default_method_resolution() {
    let mut checker = ProtocolChecker::new();
    checker.register_protocol(create_eq_protocol());

    // Create implementation for Int (only implements eq, not ne)
    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );
    // Note: ne is NOT explicitly implemented

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Eq", Span::default())),
        protocol_args: vec![].into(),
        for_type: Type::Int,
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);

    // Resolve default method
    let resolution = checker
        .resolve_method(
            &Type::Int,
            &Path::single(Ident::new("Eq", Span::default())),
            &Text::from("ne"),
        )
        .expect("Default method should resolve");

    assert!(resolution.is_default);
    assert!(matches!(resolution.source, MethodSource::Default(_)));
}

#[test]
fn test_inherited_method_resolution() {
    let mut checker = ProtocolChecker::new();

    checker.register_protocol(create_eq_protocol());
    checker.register_protocol(create_ord_protocol());

    // Implement Ord for Int (includes Eq methods via inheritance)
    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );
    methods.insert(
        Text::from("cmp"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Int),
    );
    // Note: lt, le are NOT explicitly implemented (use defaults)

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Ord", Span::default())),
        protocol_args: vec![].into(),
        for_type: Type::Int,
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);

    // Resolve inherited default method
    let resolution = checker
        .resolve_method(
            &Type::Int,
            &Path::single(Ident::new("Ord", Span::default())),
            &Text::from("lt"),
        )
        .expect("Inherited default method should resolve");

    assert!(resolution.is_default);
}

// ==================== Test: All Methods (Including Inherited) ====================

#[test]
fn test_all_methods_includes_inherited() {
    let mut checker = ProtocolChecker::new();

    checker.register_protocol(create_eq_protocol());
    checker.register_protocol(create_ord_protocol());

    // Get all methods for Ord (should include Eq methods)
    let all_methods = checker
        .all_methods(&"Ord".into())
        .expect("Should get all methods");

    // Ord has: cmp, lt, le (3 methods)
    // Eq has: eq, ne (2 methods)
    // Total: 5 methods
    assert!(all_methods.len() >= 3, "Should have at least Ord's methods");

    // Check that Eq methods are included
    let method_names: Vec<_> = all_methods.iter().map(|m| m.name.as_str()).collect();
    assert!(method_names.contains(&"cmp"), "Should have cmp from Ord");
    assert!(method_names.contains(&"eq"), "Should have eq from Eq");
}

#[test]
fn test_method_override_in_subprotocol() {
    let mut checker = ProtocolChecker::new();

    // Create Base protocol with default method
    let mut base_methods = Map::new();
    base_methods.insert(
        Text::from("process"),
        ProtocolMethod {
            name: Text::from("process"),
            ty: Type::function(vec![Type::Int].into(), Type::Int),
            has_default: true,
            doc: Maybe::None,
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        },
    );

    let base = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Base".into(),
        type_params: List::new(),
        methods: base_methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    // Create Derived protocol that overrides process
    let mut derived_methods = Map::new();
    derived_methods.insert(
        Text::from("process"),
        ProtocolMethod {
            name: Text::from("process"),
            ty: Type::function(vec![Type::Int].into(), Type::Int),
            has_default: true, // Different implementation
            doc: Maybe::Some("Overridden in Derived".into()),
            refinement_constraints: Map::new(),
            is_async: false,
            context_requirements: List::new(),
            type_param_names: List::new(),
            type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
        },
    );

    let mut derived_supers = List::new();
    derived_supers.push(ProtocolBound {
        protocol: Path::single(Ident::new("Base", Span::default())),
        args: List::new(),
        is_negative: false,
    });

    let derived = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Derived".into(),
        type_params: List::new(),
        methods: derived_methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: derived_supers,
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    checker.register_protocol(base);
    checker.register_protocol(derived);

    let all_methods = checker
        .all_methods(&"Derived".into())
        .expect("Should get all methods");

    // Should have only one "process" method (overridden)
    let process_methods: Vec<_> = all_methods.iter().filter(|m| m.name == "process").collect();
    assert_eq!(
        process_methods.len(),
        1,
        "Should have exactly one process method"
    );
    assert_eq!(
        process_methods[0].doc,
        Maybe::Some("Overridden in Derived".into())
    );
}

// ==================== Test: Protocol Satisfaction ====================

#[test]
fn test_protocol_satisfaction_explicit_impl() {
    let mut checker = ProtocolChecker::new();
    checker.register_protocol(create_eq_protocol());

    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Eq", Span::default())),
        protocol_args: vec![].into(),
        for_type: Type::Int,
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);

    let satisfied = checker
        .check_protocol_satisfied(&Type::Int, &"Eq".into())
        .expect("Check should succeed");

    assert!(satisfied);
}

#[test]
fn test_protocol_satisfaction_via_inheritance() {
    let mut checker = ProtocolChecker::new();

    checker.register_protocol(create_eq_protocol());
    checker.register_protocol(create_ord_protocol());

    // Implement Ord for Int
    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );
    methods.insert(
        Text::from("cmp"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Int),
    );

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Ord", Span::default())),
        protocol_args: vec![].into(),
        for_type: Type::Int,
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);

    // Should satisfy both Ord and Eq
    assert!(
        checker
            .check_protocol_satisfied(&Type::Int, &"Ord".into())
            .unwrap()
    );
    assert!(
        checker
            .check_protocol_satisfied(&Type::Int, &"Eq".into())
            .unwrap()
    );
}

// ==================== Test: VTable Generation ====================

#[test]
fn test_vtable_generation_basic() {
    let mut checker = ProtocolChecker::new_empty();
    checker.register_protocol(create_eq_protocol());

    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );
    methods.insert(
        Text::from("ne"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Eq", Span::default())),
        protocol_args: vec![].into(),
        for_type: Type::Int,
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);

    let vtable = checker
        .generate_vtable(&Type::Int, &Path::single(Ident::new("Eq", Span::default())))
        .expect("VTable generation should succeed");

    assert_eq!(vtable.protocol, "Eq");
    assert_eq!(vtable.for_type, Type::Int);
    assert_eq!(vtable.method_count, 2); // eq and ne

    // Check method indices
    assert!(vtable.get_method_index(&Text::from("eq")).is_some());
    assert!(vtable.get_method_index(&Text::from("ne")).is_some());
}

#[test]
fn test_vtable_with_default_methods() {
    let mut checker = ProtocolChecker::new_empty();
    checker.register_protocol(create_eq_protocol());

    // Only implement eq, not ne (uses default)
    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Eq", Span::default())),
        protocol_args: vec![].into(),
        for_type: Type::Int,
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);

    let vtable = checker
        .generate_vtable(&Type::Int, &Path::single(Ident::new("Eq", Span::default())))
        .expect("VTable generation should succeed with default methods");

    // VTable should include both eq and ne (default)
    assert_eq!(vtable.method_count, 2);
}

#[test]
fn test_vtable_layout_info() {
    let mut checker = ProtocolChecker::new_empty();
    checker.register_protocol(create_eq_protocol());

    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );
    methods.insert(
        Text::from("ne"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Eq", Span::default())),
        protocol_args: vec![].into(),
        for_type: Type::Int,
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);

    let vtable = checker
        .generate_vtable(&Type::Int, &Path::single(Ident::new("Eq", Span::default())))
        .expect("VTable generation should succeed");

    let layout = vtable.layout_info();

    // 2 methods * 8 bytes per function pointer = 16 bytes
    assert_eq!(layout.size, 16);
    assert_eq!(layout.alignment, 8);

    // Check offsets
    assert!(layout.method_offsets.get(&Text::from("eq")).is_some());
    assert!(layout.method_offsets.get(&Text::from("ne")).is_some());
}

// ==================== Test: Associated Types ====================

#[test]
fn test_associated_type_declaration() {
    let mut checker = ProtocolChecker::new();
    let iterator = create_iterator_protocol();

    // Iterator should have Item associated type
    assert!(iterator.associated_types.contains_key(&"Item".into()));

    checker.register_protocol(iterator);
}

#[test]
fn test_associated_type_in_implementation() {
    let mut checker = ProtocolChecker::new();
    checker.register_protocol(create_iterator_protocol());

    let mut methods = Map::new();
    methods.insert(
        Text::from("next"),
        Type::function(vec![Type::Int].into(), Type::Int),
    );

    let mut associated_types = Map::new();
    associated_types.insert("Item".into(), Type::Int); // Concrete type for Item

    // Use a simpler type since List might not be available in Type
    let for_type = Type::Int;

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Iterator", Span::default())),
        protocol_args: vec![].into(),
        for_type: for_type.clone(),
        where_clauses: vec![].into(),
        methods,
        associated_types,
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);

    // Verify implementation is registered
    assert!(checker.implements(
        &for_type,
        &Path::single(Ident::new("Iterator", Span::default()))
    ));
}

// ==================== Test: Error Cases ====================

#[test]
fn test_missing_required_method_error() {
    let mut checker = ProtocolChecker::new();
    checker.register_protocol(create_eq_protocol());

    // Implement without required method
    let methods = Map::new(); // Empty - missing required 'eq'

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Eq", Span::default())),
        protocol_args: vec![].into(),
        for_type: Type::Int,
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);

    // VTable generation should fail
    let result =
        checker.generate_vtable(&Type::Int, &Path::single(Ident::new("Eq", Span::default())));

    assert!(result.is_err());
}

#[test]
fn test_protocol_not_found_error() {
    let checker = ProtocolChecker::new();

    let result = checker.all_methods(&"NonExistent".into());
    assert!(result.is_err());
}

#[test]
fn test_implementation_not_found_error() {
    let mut checker = ProtocolChecker::new();
    checker.register_protocol(create_eq_protocol());

    // No implementation registered
    let result = checker.resolve_method(
        &Type::Int,
        &Path::single(Ident::new("Eq", Span::default())),
        &Text::from("eq"),
    );

    assert!(result.is_err());
}

// ==================== Test: Protocol Names Listing ====================

#[test]
fn test_protocol_names_listing() {
    let checker = ProtocolChecker::new();

    let names = checker.protocol_names();

    // Should have standard protocols
    assert!(names.iter().any(|n| n == "Eq"));
    assert!(names.iter().any(|n| n == "Ord"));
    assert!(names.iter().any(|n| n == "Show"));
}

// ==================== Test: Performance Characteristics ====================

#[test]
fn test_vtable_performance_layout() {
    let mut checker = ProtocolChecker::new_empty();
    checker.register_protocol(create_eq_protocol());
    checker.register_protocol(create_ord_protocol());

    let mut methods = Map::new();
    methods.insert(
        Text::from("eq"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );
    methods.insert(
        Text::from("cmp"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Int),
    );
    methods.insert(
        Text::from("lt"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );
    methods.insert(
        Text::from("le"),
        Type::function(vec![Type::Int, Type::Int].into(), Type::Bool),
    );

    let impl_ = ProtocolImpl {
        protocol: Path::single(Ident::new("Ord", Span::default())),
        protocol_args: vec![].into(),
        for_type: Type::Int,
        where_clauses: vec![].into(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,

        impl_crate: Maybe::None,
        span: Span::default(),
        type_param_fn_bounds: Map::new(),
    };

    checker.register_impl(impl_);

    let vtable = checker
        .generate_vtable(
            &Type::Int,
            &Path::single(Ident::new("Ord", Span::default())),
        )
        .expect("VTable generation should succeed");

    let layout = vtable.layout_info();

    // Verify cache-friendly alignment (8-byte aligned)
    assert_eq!(layout.alignment, 8);

    // Verify compact layout (no wasted space)
    assert_eq!(layout.size, vtable.method_count * 8);
}

// ==================== Test: Complex Scenarios ====================

#[test]
fn test_multiple_inheritance_levels() {
    let mut checker = ProtocolChecker::new();

    // Level 1: Base
    let base = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Base".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    // Level 2: Middle (inherits Base)
    let mut middle_supers = List::new();
    middle_supers.push(ProtocolBound {
        protocol: Path::single(Ident::new("Base", Span::default())),
        args: List::new(),
        is_negative: false,
    });

    let middle = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Middle".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: middle_supers,
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    // Level 3: Top (inherits Middle)
    let mut top_supers = List::new();
    top_supers.push(ProtocolBound {
        protocol: Path::single(Ident::new("Middle", Span::default())),
        args: List::new(),
        is_negative: false,
    });

    let top = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Top".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: top_supers,
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    checker.register_protocol(base);
    checker.register_protocol(middle);
    checker.register_protocol(top);

    // Verify transitive inheritance
    assert!(checker.inherits_from(&"Top".into(), &"Base".into()));
    assert!(checker.inherits_from(&"Top".into(), &"Middle".into()));
    assert!(checker.inherits_from(&"Middle".into(), &"Base".into()));
}

#[test]
fn test_diamond_inheritance_pattern() {
    let mut checker = ProtocolChecker::new();

    // Base protocol
    let base = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Base".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    // Left and Right both inherit Base
    let mut left_supers = List::new();
    left_supers.push(ProtocolBound {
        protocol: Path::single(Ident::new("Base", Span::default())),
        args: List::new(),
        is_negative: false,
    });

    let left = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Left".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: left_supers,
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    let mut right_supers = List::new();
    right_supers.push(ProtocolBound {
        protocol: Path::single(Ident::new("Base", Span::default())),
        args: List::new(),
        is_negative: false,
    });

    let right = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Right".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: right_supers,
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    // Diamond inherits both Left and Right
    let mut diamond_supers = List::new();
    diamond_supers.push(ProtocolBound {
        protocol: Path::single(Ident::new("Left", Span::default())),
        args: List::new(),
        is_negative: false,
    });
    diamond_supers.push(ProtocolBound {
        protocol: Path::single(Ident::new("Right", Span::default())),
        args: List::new(),
        is_negative: false,
    });

    let diamond = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Diamond".into(),
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: diamond_supers,
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    checker.register_protocol(base);
    checker.register_protocol(left);
    checker.register_protocol(right);
    checker.register_protocol(diamond);

    // Verify diamond inherits Base through both paths
    assert!(checker.inherits_from(&"Diamond".into(), &"Base".into()));
    assert!(checker.inherits_from(&"Diamond".into(), &"Left".into()));
    assert!(checker.inherits_from(&"Diamond".into(), &"Right".into()));

    // Verify no duplicate methods due to diamond
    let all_methods = checker
        .all_methods(&"Diamond".into())
        .expect("Should handle diamond inheritance");

    // Should not have duplicate methods
    let mut seen = std::collections::HashSet::new();
    for method in all_methods.iter() {
        assert!(
            seen.insert(&method.name),
            "Duplicate method: {}",
            method.name
        );
    }
}
