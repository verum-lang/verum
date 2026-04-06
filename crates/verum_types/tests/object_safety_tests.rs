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
// Tests for object safety validation
// Basic protocols with simple associated types (initial release) — 2 lines 11618-11759

use verum_ast::span::Span;
use verum_common::{List, Map, Maybe, Text};
use verum_types::protocol::{
    AssociatedConst, ObjectSafetyError, Protocol, ProtocolChecker, ProtocolMethod,
};
use verum_types::ty::Type;

#[test]
fn test_object_safe_simple_protocol() {
    // Object-safe protocol:
    // protocol Draw {
    //     fn draw(&self);
    // }

    let mut methods = Map::new();
    methods.insert(
        Text::from("draw"),
        ProtocolMethod {
            name: Text::from("draw"),
            ty: Type::function(
                vec![Type::reference(false, Type::int())].into(), // &self (simplified)
                Type::unit(),
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

    let draw_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Draw".into(),
        type_params: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test_crate")),
        span: Span::default(),
    };

    let mut checker = ProtocolChecker::new();
    checker.register_protocol(draw_protocol);

    // Should be object-safe
    let result = checker.check_object_safety(&"Draw".into());
    assert!(result.is_ok());
}

#[test]
fn test_not_object_safe_returns_self() {
    // NOT object-safe:
    // protocol Clone {
    //     fn clone(&self) -> Self;  // ERROR: returns Self
    // }

    let mut methods = Map::new();
    methods.insert(
        Text::from("clone"),
        ProtocolMethod {
            name: Text::from("clone"),
            ty: Type::function(
                vec![Type::reference(false, Type::int())].into(), // &self (simplified)
                // Return Self type (represented as Named { path: "Self" })
                Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                        "Self",
                        Span::default(),
                    )),
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

    let clone_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Clone".into(),
        type_params: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test_crate")),
        span: Span::default(),
    };

    let mut checker = ProtocolChecker::new();
    checker.register_protocol(clone_protocol);

    // Should NOT be object-safe
    let result = checker.check_object_safety(&"Clone".into());
    assert!(result.is_err());

    if let Err(errors) = result {
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], ObjectSafetyError::ReturnsSelf { .. }));
    }
}

#[test]
fn test_not_object_safe_generic_method() {
    // NOT object-safe:
    // protocol Generic {
    //     fn method<T>(&self, x: T);  // ERROR: generic method
    // }

    let mut methods = Map::new();
    methods.insert(
        Text::from("method"),
        ProtocolMethod {
            name: Text::from("method"),
            ty: Type::Function {
                params: vec![
                    Type::reference(false, Type::int()),             // &self
                    Type::Var(verum_types::ty::TypeVar::with_id(0)), // T
                ]
                .into(),
                return_type: Box::new(Type::unit()),
                type_params: vec![verum_types::context::TypeParam::new("T", Span::default())]
                    .into(),
                contexts: None,
                properties: None,
            },
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

    let generic_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Generic".into(),
        type_params: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test_crate")),
        span: Span::default(),
    };

    let mut checker = ProtocolChecker::new();
    checker.register_protocol(generic_protocol);

    // Should NOT be object-safe
    let result = checker.check_object_safety(&"Generic".into());
    assert!(result.is_err());

    if let Err(errors) = result {
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ObjectSafetyError::GenericMethod { .. }))
        );
    }
}

#[test]
fn test_not_object_safe_no_self_parameter() {
    // NOT object-safe:
    // protocol Static {
    //     fn new() -> Self;  // ERROR: no self parameter
    // }

    let mut methods = Map::new();
    methods.insert(
        Text::from("new"),
        ProtocolMethod {
            name: Text::from("new"),
            ty: Type::function(
                vec![].into(), // No parameters
                Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                        "Self",
                        Span::default(),
                    )),
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

    let static_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Static".into(),
        type_params: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test_crate")),
        span: Span::default(),
    };

    let mut checker = ProtocolChecker::new();
    checker.register_protocol(static_protocol);

    // Should NOT be object-safe
    let result = checker.check_object_safety(&"Static".into());
    assert!(result.is_err());

    if let Err(errors) = result {
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ObjectSafetyError::NoSelfParameter { .. }))
        );
        // Also returns Self, so might have two errors
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ObjectSafetyError::ReturnsSelf { .. }))
        );
    }
}

#[test]
fn test_not_object_safe_takes_self_by_value() {
    // NOT object-safe:
    // protocol Consume {
    //     fn consume(self);  // ERROR: takes self by value
    // }

    let mut methods = Map::new();
    methods.insert(
        Text::from("consume"),
        ProtocolMethod {
            name: Text::from("consume"),
            ty: Type::function(
                vec![Type::int()].into(), // self by value (not &self)
                Type::unit(),
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

    let consume_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Consume".into(),
        type_params: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test_crate")),
        span: Span::default(),
    };

    let mut checker = ProtocolChecker::new();
    checker.register_protocol(consume_protocol);

    // Should NOT be object-safe
    let result = checker.check_object_safety(&"Consume".into());
    assert!(result.is_err());

    if let Err(errors) = result {
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ObjectSafetyError::TakesSelfByValue { .. }))
        );
    }
}

#[test]
fn test_not_object_safe_has_associated_const() {
    // NOT object-safe:
    // protocol WithConst {
    //     const MAX: Int;
    //     fn get_max(&self) -> Int;
    // }

    let mut methods = Map::new();
    methods.insert(
        Text::from("get_max"),
        ProtocolMethod {
            name: Text::from("get_max"),
            ty: Type::function(
                vec![Type::reference(false, Type::int())].into(), // &self
                Type::int(),
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

    let mut associated_consts = Map::new();
    associated_consts.insert(
        "MAX".into(),
        AssociatedConst {
            name: "MAX".into(),
            ty: Type::int(),
        },
    );

    let const_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "WithConst".into(),
        type_params: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts,
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test_crate")),
        span: Span::default(),
    };

    let mut checker = ProtocolChecker::new();
    checker.register_protocol(const_protocol);

    // Should NOT be object-safe
    let result = checker.check_object_safety(&"WithConst".into());
    assert!(result.is_err());

    if let Err(errors) = result {
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ObjectSafetyError::HasAssociatedConst { .. }))
        );
    }
}

#[test]
fn test_object_safe_multiple_methods() {
    // Object-safe protocol with multiple methods:
    // protocol Shape {
    //     fn area(&self) -> Float;
    //     fn perimeter(&self) -> Float;
    //     fn draw(&mut self);
    // }

    let mut methods = Map::new();
    methods.insert(
        Text::from("area"),
        ProtocolMethod {
            name: Text::from("area"),
            ty: Type::function(
                vec![Type::reference(false, Type::int())].into(), // &self
                Type::float(),
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
    methods.insert(
        Text::from("perimeter"),
        ProtocolMethod {
            name: Text::from("perimeter"),
            ty: Type::function(
                vec![Type::reference(false, Type::int())].into(), // &self
                Type::float(),
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
    methods.insert(
        Text::from("draw"),
        ProtocolMethod {
            name: Text::from("draw"),
            ty: Type::function(
                vec![Type::reference(true, Type::int())].into(), // &mut self
                Type::unit(),
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

    let shape_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Shape".into(),
        type_params: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test_crate")),
        span: Span::default(),
    };

    let mut checker = ProtocolChecker::new();
    checker.register_protocol(shape_protocol);

    // Should be object-safe
    let result = checker.check_object_safety(&"Shape".into());
    assert!(result.is_ok());
}

#[test]
fn test_object_safe_checked_and_unsafe_references() {
    // Test that checked and unsafe references are also considered object-safe

    let mut methods = Map::new();
    methods.insert(
        Text::from("method_checked"),
        ProtocolMethod {
            name: Text::from("method_checked"),
            ty: Type::function(
                vec![Type::checked_reference(false, Type::int())].into(), // &checked self
                Type::unit(),
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
    methods.insert(
        Text::from("method_unsafe"),
        ProtocolMethod {
            name: Text::from("method_unsafe"),
            ty: Type::function(
                vec![Type::unsafe_reference(false, Type::int())].into(), // &unsafe self
                Type::unit(),
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

    let protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "MultiRef".into(),
        type_params: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test_crate")),
        span: Span::default(),
    };

    let mut checker = ProtocolChecker::new();
    checker.register_protocol(protocol);

    // Should be object-safe (all reference types are allowed)
    let result = checker.check_object_safety(&"MultiRef".into());
    assert!(result.is_ok());
}
