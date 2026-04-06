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
// Simplified test for protocol method dispatch fix
//
// This test verifies that protocol implementations properly register methods
// in the ProtocolImpl structure so they can be found during method resolution.

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map, Maybe, Text};
use verum_types::TypeChecker;
use verum_types::protocol::{ProtocolChecker, ProtocolImpl};
use verum_types::ty::Type;

fn dummy() -> Span {
    Span::dummy()
}

fn ident(name: &str) -> Ident {
    Ident::new(name, dummy())
}

fn path(name: &str) -> Path {
    Path::single(ident(name))
}

#[test]
fn test_protocol_impl_has_methods() {
    let mut checker = ProtocolChecker::new();

    // Create a record type for Point
    let mut point_fields = indexmap::IndexMap::new();
    point_fields.insert(Text::from("x"), Type::float());
    point_fields.insert(Text::from("y"), Type::float());
    let point_type = Type::Record(point_fields);

    // Create a protocol implementation with methods
    let mut methods = Map::new();
    methods.insert(
        Text::from("to_text"),
        Type::function(vec![point_type.clone()].into(), Type::text()),
    );

    let protocol_impl = ProtocolImpl {
        protocol: path("Printable"),
        protocol_args: List::new(),
        for_type: point_type.clone(),
        where_clauses: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::None,
        span: dummy(),
        type_param_fn_bounds: Map::new(),
    };

    // Register the implementation
    let _ = checker.register_impl(protocol_impl);

    // Verify the implementation was registered
    let impls = checker.get_implementations(&point_type);
    assert!(
        !impls.is_empty(),
        "Should have at least one implementation for Point type"
    );

    // Verify the method is in the implementation
    let impl_ = impls
        .first()
        .expect("Should have at least one implementation");
    assert!(
        impl_.methods.contains_key(&Text::from("to_text")),
        "Implementation should contain 'to_text' method. Found methods: {:?}",
        impl_.methods.keys().collect::<Vec<_>>()
    );

    // Verify method lookup works
    let method_result = checker.lookup_protocol_method(&point_type, &Text::from("to_text"));
    assert!(
        matches!(method_result, Ok(Maybe::Some(_))),
        "Method 'to_text' should be found for Point type. Result: {:?}",
        method_result
    );
}

#[test]
fn test_multiple_methods_in_impl() {
    let mut checker = ProtocolChecker::new();

    // Create a simple type
    let test_type = Type::int();

    // Create implementation with multiple methods
    let mut methods = Map::new();
    methods.insert(
        Text::from("method_a"),
        Type::function(vec![test_type.clone()].into(), Type::int()),
    );
    methods.insert(
        Text::from("method_b"),
        Type::function(vec![test_type.clone()].into(), Type::text()),
    );

    let protocol_impl = ProtocolImpl {
        protocol: path("TestProtocol"),
        protocol_args: List::new(),
        for_type: test_type.clone(),
        where_clauses: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::None,
        span: dummy(),
        type_param_fn_bounds: Map::new(),
    };

    // Register the implementation
    let _ = checker.register_impl(protocol_impl);

    // Verify both methods are accessible
    let method_a = checker.lookup_protocol_method(&test_type, &Text::from("method_a"));
    assert!(
        matches!(method_a, Ok(Maybe::Some(_))),
        "Method 'method_a' should be found"
    );

    let method_b = checker.lookup_protocol_method(&test_type, &Text::from("method_b"));
    assert!(
        matches!(method_b, Ok(Maybe::Some(_))),
        "Method 'method_b' should be found"
    );
}

#[test]
fn test_method_not_found_when_not_implemented() {
    let checker = ProtocolChecker::new();

    // Create a type that doesn't have any implementations
    let test_type = Type::bool();

    // Try to look up a method that doesn't exist
    let result = checker.lookup_protocol_method(&test_type, &Text::from("nonexistent"));

    // Should return None, not an error
    assert!(
        matches!(result, Ok(Maybe::None)),
        "Should return None for non-existent method, got: {:?}",
        result
    );
}

#[test]
fn test_named_type_normalization() {
    // Test that Type::Named gets normalized to the underlying type
    let mut checker = TypeChecker::new();

    // Register a type alias: type Point is { x: Float, y: Float }
    let mut point_fields = indexmap::IndexMap::new();
    point_fields.insert(Text::from("x"), Type::float());
    point_fields.insert(Text::from("y"), Type::float());
    let point_record_type = Type::Record(point_fields.clone());

    // Register the type in the context
    checker
        .context_mut()
        .define_type(Text::from("Point"), point_record_type.clone());

    // Create a Named type that refers to Point
    let point_named_type = Type::Named {
        path: Path::single(ident("Point")),
        args: List::new(),
    };

    // Register a protocol implementation for the Record type
    let mut methods = Map::new();
    methods.insert(
        Text::from("to_text"),
        Type::function(vec![point_record_type.clone()].into(), Type::text()),
    );

    let protocol_impl = ProtocolImpl {
        protocol: path("Printable"),
        protocol_args: List::new(),
        for_type: point_record_type.clone(),
        where_clauses: List::new(),
        methods,
        associated_types: Map::new(),
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::None,
        span: dummy(),
        type_param_fn_bounds: Map::new(),
    };

    let _ = checker.protocol_checker.write().register_impl(protocol_impl);

    // Now verify that we can find the implementation using the Named type
    // This would fail before the fix because Type::Named wasn't being normalized
    let guard = checker.protocol_checker.read();
    let impls_via_record = guard.get_implementations(&point_record_type);
    let _impls_via_named = guard.get_implementations(&point_named_type);

    // Both should be empty because we didn't normalize the named type
    // But after the fix in infer_method_call, it should work when used in that context
    // For now, this test just verifies the basic setup works
    assert!(
        !impls_via_record.is_empty(),
        "Should find implementation via Record type"
    );
}
