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
//! Comprehensive tests for protocol SMT encoding
//!
//! Tests cover:
//! - Protocol implementation checking
//! - Associated type resolution
//! - Protocol hierarchy verification
//! - Coherence checking
//! - Cycle detection
//! - Caching behavior

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, PathSegment, Type};
use verum_protocol_types::protocol_base::{AssociatedType, Protocol, ProtocolBound, ProtocolImpl};
use verum_protocol_types::specialization::SpecializationInfo;
use verum_smt::protocol_smt::{
    ProtocolEncoder, ProtocolError, check_implements, verify_coherence, verify_hierarchy,
};
use verum_common::{List, Map, Maybe, Text};

fn create_simple_protocol(name: &str) -> Protocol {
    Protocol {
        name: name.into(),
        type_params: List::new(),
        super_protocols: List::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        methods: Map::new(),
        defining_crate: Maybe::None,
        span: Span::default(),
    }
}

fn create_simple_path(name: &str) -> Path {
    let ident = Ident {
        name: name.into(),
        span: Span::dummy(),
    };
    Path {
        segments: vec![PathSegment::Name(ident)].into(),
        span: Span::dummy(),
    }
}

fn create_simple_impl(ty: Type, protocol_name: &str) -> ProtocolImpl {
    ProtocolImpl {
        for_type: ty,
        protocol: create_simple_path(protocol_name),
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
fn test_encoder_creation() {
    let encoder = ProtocolEncoder::new();
    assert_eq!(encoder.stats().protocol_checks, 0);
    assert_eq!(encoder.stats().hierarchy_checks, 0);
}

#[test]
fn test_register_protocol() {
    let mut encoder = ProtocolEncoder::new();
    let protocol = create_simple_protocol("Display");

    encoder.register_protocol(protocol);
    assert_eq!(encoder.stats().hierarchy_checks, 1);
}

#[test]
fn test_register_implementation() {
    let mut encoder = ProtocolEncoder::new();
    let impl_ = create_simple_impl(Type::int(Span::dummy()), "Display");

    encoder.register_implementation(impl_);
}

#[test]
fn test_verify_coherence_empty() {
    let encoder = ProtocolEncoder::new();
    let result = encoder.verify_coherence();
    assert!(result.is_ok());
}

#[test]
fn test_verify_coherence_single_impl() {
    let mut encoder = ProtocolEncoder::new();
    let impl_ = create_simple_impl(Type::int(Span::dummy()), "Display");
    encoder.register_implementation(impl_);

    let result = encoder.verify_coherence();
    assert!(result.is_ok());
}

#[test]
fn test_hierarchy_cycle_detection_empty() {
    let encoder = ProtocolEncoder::new();
    let result = encoder.check_hierarchy_cycles();
    assert!(result.is_ok());
}

#[test]
fn test_hierarchy_cycle_detection_single_protocol() {
    let mut encoder = ProtocolEncoder::new();
    let protocol = create_simple_protocol("Display");
    encoder.register_protocol(protocol);

    let result = encoder.check_hierarchy_cycles();
    assert!(result.is_ok());
}

#[test]
fn test_cache_clearing() {
    let mut encoder = ProtocolEncoder::new();

    // Add a cache entry (simulated)
    encoder.clear_cache();
    assert_eq!(encoder.stats().protocol_checks, 0);
}

#[test]
fn test_encoder_stats() {
    let mut encoder = ProtocolEncoder::new();

    let protocol = create_simple_protocol("Debug");
    encoder.register_protocol(protocol);

    let stats = encoder.stats();
    assert_eq!(stats.hierarchy_checks, 1);
    assert_eq!(stats.protocol_checks, 0);
}

#[test]
fn test_verify_hierarchy_empty() {
    let result = verify_hierarchy(&[]);
    assert!(result.is_ok());
}

#[test]
fn test_verify_hierarchy_single_protocol() {
    let protocol = create_simple_protocol("Clone");
    let result = verify_hierarchy(&[protocol]);
    assert!(result.is_ok());
}

#[test]
fn test_verify_coherence_function() {
    let result = verify_coherence(&[]);
    assert!(result.is_ok());
}

#[test]
fn test_verify_coherence_with_impl() {
    let impl_ = create_simple_impl(Type::int(Span::dummy()), "Display");
    let result = verify_coherence(&[impl_]);
    assert!(result.is_ok());
}

#[test]
fn test_default_encoder() {
    let encoder = ProtocolEncoder::default();
    assert_eq!(encoder.stats().protocol_checks, 0);
}

#[test]
fn test_multiple_protocol_registration() {
    let mut encoder = ProtocolEncoder::new();

    encoder.register_protocol(create_simple_protocol("Display"));
    encoder.register_protocol(create_simple_protocol("Debug"));
    encoder.register_protocol(create_simple_protocol("Clone"));

    assert_eq!(encoder.stats().hierarchy_checks, 3);
}

#[test]
fn test_multiple_impl_registration() {
    let mut encoder = ProtocolEncoder::new();

    encoder.register_implementation(create_simple_impl(Type::int(Span::dummy()), "Display"));
    encoder.register_implementation(create_simple_impl(Type::bool(Span::dummy()), "Display"));
    // Registering a duplicate impl for Int:Display should cause coherence violation
    encoder.register_implementation(create_simple_impl(Type::int(Span::dummy()), "Display"));

    let result = encoder.verify_coherence();
    // Should fail because Int implements Display twice
    assert!(
        result.is_err(),
        "Registering the same implementation twice should fail coherence check"
    );
}

#[test]
fn test_protocol_with_superprotocol() {
    let mut encoder = ProtocolEncoder::new();

    let mut sub_protocol = create_simple_protocol("Eq");
    // Would add superprotocol here in full implementation

    encoder.register_protocol(sub_protocol);
    let result = encoder.check_hierarchy_cycles();
    assert!(result.is_ok());
}

#[test]
fn test_associated_type_resolution_empty() {
    let encoder = ProtocolEncoder::new();

    // Should fail - no implementations registered
    let result = encoder.resolve_associated_type(&Type::int(Span::dummy()), "Iterator", "Item");
    assert!(result.is_err());
}

#[test]
fn test_implementation_with_associated_type() {
    let mut encoder = ProtocolEncoder::new();

    let mut impl_ = create_simple_impl(Type::int(Span::dummy()), "Iterator");
    impl_
        .associated_types
        .insert(Text::from("Item"), Type::int(Span::dummy()));

    encoder.register_implementation(impl_);

    // Now should succeed
    let result = encoder.resolve_associated_type(&Type::int(Span::dummy()), "Iterator", "Item");
    assert!(result.is_ok());
}

#[test]
fn test_cache_behavior() {
    let mut encoder = ProtocolEncoder::new();

    // Register impl
    encoder.register_implementation(create_simple_impl(Type::int(Span::dummy()), "Display"));

    // First check
    let _ = encoder.check_implements(&Type::int(Span::dummy()), "Display");

    // Second check should hit cache
    let _ = encoder.check_implements(&Type::int(Span::dummy()), "Display");
}
