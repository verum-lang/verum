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
// Comprehensive tests for protocol coherence checking (orphan rule and overlap detection)
//
// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .6 - Coherence Rules (lines 10746-10966)
//
// Tests cover:
// 1. Orphan rule validation
// 2. Overlap detection
// 3. Error message quality
// 4. Edge cases and corner cases

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map, Text};
use verum_types::protocol::{CoherenceError, ProtocolChecker, ProtocolImpl};
use verum_types::ty::Type;

// ==================== Helper Functions ====================

/// Create a test protocol implementation
fn make_impl(protocol: &str, for_type: Type, impl_crate: Option<&str>) -> ProtocolImpl {
    use verum_common::Maybe;
    ProtocolImpl {
        protocol: Path::single(Ident::new(protocol, Span::default())),
        protocol_args: vec![].into(),
        for_type,
        where_clauses: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        span: Span::default(),
        impl_crate: match impl_crate {
            Some(s) => Maybe::Some(s.into()),
            None => Maybe::None,
        },
        type_param_fn_bounds: Map::new(),
    }
}

/// Create a named type
fn named_type(name: &str) -> Type {
    Type::Named {
        path: Path::single(Ident::new(name, Span::default())),
        args: vec![].into(),
    }
}

/// Create a generic type
fn generic_type(name: &str, args: Vec<Type>) -> Type {
    Type::Named {
        path: Path::single(Ident::new(name, Span::default())),
        args: args.into(),
    }
}

// ==================== Orphan Rule Tests ====================

#[test]
fn test_orphan_rule_local_protocol_foreign_type() {
    // Orphan rules: local protocol + foreign type is allowed (implementing your protocol for external types)
    // OK: Protocol is local
    // implement MyProtocol for Int { ... }

    let mut checker = ProtocolChecker::new();
    checker.set_current_crate(Text::from("my_crate"));

    // Register local protocol
    let protocol = verum_types::protocol::Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "MyProtocol".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Option::None,
        span: Span::default(),
        defining_crate: Option::Some(Text::from("my_crate")),
    };
    checker.register_protocol(protocol);

    // Implement local protocol for foreign type (Int from verum_std)
    let impl_ = make_impl("MyProtocol", Type::Int, Some("my_crate"));

    // Should succeed - protocol is local
    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_ok(),
        "Local protocol for foreign type should be allowed"
    );
}

#[test]
fn test_orphan_rule_foreign_protocol_local_type() {
    // Orphan rules: foreign protocol + local type is allowed (implementing external protocol for your types)
    // OK: Type is local
    // implement Display for MyLocalType { ... }

    let mut checker = ProtocolChecker::new();
    checker.set_current_crate(Text::from("my_crate"));

    // Register local type
    checker.register_type_crate("MyType".into(), Text::from("my_crate"));

    // Implement foreign protocol (Show from verum_std) for local type
    let impl_ = make_impl("Show", named_type("MyType"), Some("my_crate"));

    // Should succeed - type is local
    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_ok(),
        "Foreign protocol for local type should be allowed"
    );
}

#[test]
fn test_orphan_rule_violation_both_foreign() {
    // Orphan rules: foreign protocol + foreign type is NOT allowed (cannot implement external protocol for external type)
    // ERROR: Neither Display nor Text is local
    // implement Display for Text { ... }

    let mut checker = ProtocolChecker::new();
    checker.set_current_crate(Text::from("my_crate"));

    // Try to implement foreign protocol for foreign type
    let impl_ = make_impl("Show", Type::Text, Some("my_crate"));

    // Should fail - orphan rule violation
    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_err(),
        "Foreign protocol for foreign type should be rejected"
    );

    match result {
        Err(CoherenceError::OrphanRuleViolation {
            protocol,
            for_type,
            current_crate,
            newtype_suggestion,
            local_protocol_suggestion,
            ..
        }) => {
            assert_eq!(protocol.as_str(), "Show");
            assert_eq!(for_type.as_str(), "Text");
            assert_eq!(current_crate.as_str(), "my_crate");
            assert!(newtype_suggestion.contains("type MyText(Text)"));
            assert!(local_protocol_suggestion.contains("protocol MyShow"));
        }
        _ => panic!("Expected OrphanRuleViolation error"),
    }
}

#[test]
fn test_orphan_rule_generic_with_local_type_param() {
    // Orphan rules: type parameter makes the implementing type "local" for coherence purposes
    // OK: Type parameter makes it local (blanket implementation)
    // implement<T> MyProtocol for List<T> where type T: SomeProtocol { ... }

    let mut checker = ProtocolChecker::new();
    checker.set_current_crate(Text::from("my_crate"));

    // Register local protocol
    let protocol = verum_types::protocol::Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "MyProtocol".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Option::None,
        span: Span::default(),
        defining_crate: Option::Some(Text::from("my_crate")),
    };
    checker.register_protocol(protocol);

    // Register local type
    checker.register_type_crate("MyType".into(), Text::from("my_crate"));

    // Implement local protocol for generic List with local type parameter
    let impl_ = make_impl(
        "MyProtocol",
        generic_type("List", vec![named_type("MyType")]),
        Some("my_crate"),
    );

    // Should succeed - protocol is local AND type parameter contains local type
    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_ok(),
        "Protocol for generic type with local param should be allowed"
    );
}

#[test]
fn test_orphan_rule_std_implementation() {
    // Standard library implementations should bypass orphan checking
    let checker = ProtocolChecker::new();

    // verum_std implementing Show for Int (both local to verum_std)
    let impl_ = make_impl("Show", Type::Int, Some("stdlib"));

    // Should succeed - verum_std is trusted
    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_ok(),
        "verum_std implementations should be allowed"
    );
}

// ==================== Overlap Detection Tests ====================

#[test]
fn test_overlap_same_concrete_types() {
    // Overlap detection: two implementations overlap when their types can unify
    // Two implementations for exactly the same type should overlap

    let checker = ProtocolChecker::new();

    let impl1 = make_impl("Show", Type::Int, Some("crate1"));
    let impl2 = make_impl("Show", Type::Int, Some("crate2"));

    let result = checker.check_overlap(&impl1, &impl2);
    assert!(result.is_err(), "Duplicate implementations should overlap");

    match result {
        Err(CoherenceError::OverlappingImplementations {
            protocol, for_type, ..
        }) => {
            assert_eq!(protocol.as_str(), "Show");
            assert_eq!(for_type.as_str(), "Int");
        }
        _ => panic!("Expected OverlappingImplementations error"),
    }
}

#[test]
fn test_overlap_generic_and_concrete() {
    // Specialization lattice: more specific impls override less specific ones based on type specificity
    // implement<T> Clone for List<T> { ... }
    // implement Clone for List<Int> { ... }
    // ERROR: Overlapping implementations

    let checker = ProtocolChecker::new();

    let impl1 = make_impl(
        "Clone",
        generic_type(
            "List",
            vec![Type::Var(verum_types::ty::TypeVar::with_id(0))],
        ),
        Some("crate1"),
    );
    let impl2 = make_impl(
        "Clone",
        generic_type("List", vec![Type::Int]),
        Some("crate2"),
    );

    let result = checker.check_overlap(&impl1, &impl2);
    assert!(
        result.is_err(),
        "Generic and concrete implementations should overlap"
    );
}

#[test]
fn test_no_overlap_different_protocols() {
    // Different protocols don't overlap, even for same type

    let checker = ProtocolChecker::new();

    let impl1 = make_impl("Show", Type::Int, Some("crate1"));
    let impl2 = make_impl("Eq", Type::Int, Some("crate2"));

    let result = checker.check_overlap(&impl1, &impl2);
    assert!(result.is_ok(), "Different protocols should not overlap");
}

#[test]
fn test_no_overlap_different_type_constructors() {
    // Different type constructors don't overlap
    // impl Show for List<T> vs impl Show for Map<K, V>

    let checker = ProtocolChecker::new();

    let impl1 = make_impl(
        "Show",
        generic_type(
            "List",
            vec![Type::Var(verum_types::ty::TypeVar::with_id(0))],
        ),
        Some("crate1"),
    );
    let impl2 = make_impl(
        "Show",
        generic_type(
            "Map",
            vec![
                Type::Var(verum_types::ty::TypeVar::with_id(0)),
                Type::Var(verum_types::ty::TypeVar::with_id(1)),
            ],
        ),
        Some("crate2"),
    );

    let result = checker.check_overlap(&impl1, &impl2);
    assert!(
        result.is_ok(),
        "Different type constructors should not overlap"
    );
}

#[test]
fn test_overlap_nested_generics() {
    // Nested generic types with unifiable parameters should overlap
    // impl Show for List<List<T>>
    // impl Show for List<List<Int>>

    let checker = ProtocolChecker::new();

    let impl1 = make_impl(
        "Show",
        generic_type(
            "List",
            vec![generic_type(
                "List",
                vec![Type::Var(verum_types::ty::TypeVar::with_id(0))],
            )],
        ),
        Some("crate1"),
    );
    let impl2 = make_impl(
        "Show",
        generic_type("List", vec![generic_type("List", vec![Type::Int])]),
        Some("crate2"),
    );

    let result = checker.check_overlap(&impl1, &impl2);
    assert!(
        result.is_err(),
        "Nested generics with unifiable params should overlap"
    );
}

// ==================== Integration Tests ====================

#[test]
fn test_coherence_check_full_pipeline() {
    // Test the complete coherence checking pipeline

    let mut checker = ProtocolChecker::new();
    checker.set_current_crate(Text::from("my_crate"));

    // Register local protocol
    let protocol = verum_types::protocol::Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "MyProtocol".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Option::None,
        span: Span::default(),
        defining_crate: Option::Some(Text::from("my_crate")),
    };
    checker.register_protocol(protocol);

    // First implementation should succeed
    let impl1 = make_impl("MyProtocol", Type::Int, Some("my_crate"));
    let result = checker.check_coherence(&impl1);
    assert!(result.is_ok(), "First implementation should succeed");

    // Manually add first implementation
    let _ = checker.register_impl(impl1);

    // Second overlapping implementation should fail
    let impl2 = make_impl("MyProtocol", Type::Int, Some("my_crate"));
    let result = checker.check_coherence(&impl2);
    assert!(result.is_err(), "Overlapping implementation should fail");
}

#[test]
fn test_register_impl_with_coherence() {
    // Test that register_impl performs coherence checking

    let mut checker = ProtocolChecker::new();
    checker.set_current_crate(Text::from("my_crate"));

    // Register local protocol
    let protocol = verum_types::protocol::Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "MyProtocol".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Option::None,
        span: Span::default(),
        defining_crate: Option::Some(Text::from("my_crate")),
    };
    checker.register_protocol(protocol);

    // First implementation should succeed
    let impl1 = make_impl("MyProtocol", Type::Int, Some("my_crate"));
    assert!(checker.register_impl(impl1).is_ok());

    // Duplicate implementation should fail
    let impl2 = make_impl("MyProtocol", Type::Int, Some("my_crate"));
    assert!(checker.register_impl(impl2).is_err());
}

// ==================== Edge Cases ====================

#[test]
fn test_orphan_rule_unknown_cog() {
    // Test orphan rule when implementing foreign protocol for foreign type in unknown cog

    let mut checker = ProtocolChecker::new();
    checker.set_current_crate(Text::from("unknown_cog"));

    // Implementation with foreign protocol (Show from verum_std) for foreign type (Text from verum_std)
    // in a third cog should fail orphan rule
    let impl_ = make_impl("Show", Type::Text, Some("unknown_cog"));

    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_err(),
        "Foreign protocol for foreign type should fail orphan rule"
    );
}

#[test]
fn test_overlap_with_type_variables() {
    // Two generic implementations with different type variables should overlap

    let checker = ProtocolChecker::new();

    let impl1 = make_impl(
        "Show",
        generic_type(
            "List",
            vec![Type::Var(verum_types::ty::TypeVar::with_id(0))],
        ),
        Some("crate1"),
    );
    let impl2 = make_impl(
        "Show",
        generic_type(
            "List",
            vec![Type::Var(verum_types::ty::TypeVar::with_id(1))],
        ),
        Some("crate2"),
    );

    let result = checker.check_overlap(&impl1, &impl2);
    assert!(
        result.is_err(),
        "Two generic implementations should overlap"
    );
}

#[test]
fn test_no_overlap_different_arity() {
    // Type constructors with different arities don't overlap

    let checker = ProtocolChecker::new();

    let impl1 = make_impl(
        "Show",
        Type::Tuple(List::from(vec![Type::Int])),
        Some("crate1"),
    );
    let impl2 = make_impl(
        "Show",
        Type::Tuple(List::from(vec![Type::Int, Type::Bool])),
        Some("crate2"),
    );

    let result = checker.check_overlap(&impl1, &impl2);
    assert!(result.is_ok(), "Different arity tuples should not overlap");
}

#[test]
fn test_orphan_rule_with_nested_local_type() {
    // Nested type with local type parameter should satisfy orphan rule

    let mut checker = ProtocolChecker::new();
    checker.set_current_crate(Text::from("my_crate"));
    checker.register_type_crate("MyType".into(), Text::from("my_crate"));

    // implement Show for List<List<MyType>>
    let impl_ = make_impl(
        "Show",
        generic_type(
            "List",
            vec![generic_type("List", vec![named_type("MyType")])],
        ),
        Some("my_crate"),
    );

    // Should succeed - deeply nested local type
    let result = checker.check_orphan_rule(&impl_);
    assert!(
        result.is_ok(),
        "Nested local type should satisfy orphan rule"
    );
}

#[test]
fn test_coherence_function_types() {
    // Function types should be handled correctly

    let checker = ProtocolChecker::new();

    let fn_type = Type::Function {
        params: List::from(vec![Type::Int]),
        return_type: Box::new(Type::Bool),
        contexts: None,
        type_params: List::new(),
        properties: None,
    };

    let impl1 = make_impl("Show", fn_type.clone(), Some("crate1"));
    let impl2 = make_impl("Show", fn_type, Some("crate2"));

    let result = checker.check_overlap(&impl1, &impl2);
    assert!(result.is_err(), "Same function types should overlap");
}

#[test]
fn test_coherence_array_types() {
    // Array types with same element type should overlap

    let checker = ProtocolChecker::new();

    let array_type = Type::Array {
        element: Box::new(Type::Int),
        size: Some(10),
    };

    let impl1 = make_impl("Show", array_type.clone(), Some("crate1"));
    let impl2 = make_impl("Show", array_type, Some("crate2"));

    let result = checker.check_overlap(&impl1, &impl2);
    assert!(result.is_err(), "Same array types should overlap");
}

#[test]
fn test_error_message_quality() {
    // Verify error messages contain helpful information

    let mut checker = ProtocolChecker::new();
    checker.set_current_crate(Text::from("my_crate"));

    // Orphan rule violation
    let impl_ = make_impl("Show", Type::Text, Some("my_crate"));
    let result = checker.check_orphan_rule(&impl_);

    match result {
        Err(e) => {
            let error_msg = format!("{}", e);
            assert!(error_msg.contains("Cannot implement protocol"));
            assert!(error_msg.contains("Show"));
            assert!(error_msg.contains("Text"));
            assert!(error_msg.contains("orphan rule"));
            assert!(error_msg.contains("newtype pattern"));
        }
        Ok(_) => panic!("Expected error"),
    }
}
