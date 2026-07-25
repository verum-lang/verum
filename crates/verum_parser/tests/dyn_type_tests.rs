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
//! Tests for dynamic protocol types (dyn Display, dyn Iterator<Item = Int>, etc.)
//!
//! Tests for dyn type syntax: dyn Protocol + Bound, with associated type bindings
//! Tests for syntax grammar compliance

use verum_ast::{
    FileId, TypeKind,
    ty::{GenericArg, TypeBoundKind},
};
use verum_parser::VerumParser;

/// Helper to parse a type from source code
fn parse_type(source: &str) -> verum_ast::Type {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_type_str(source, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse type: {}", source))
}

#[test]
fn test_dyn_single_protocol() {
    let source = "dyn Display";
    let ty = parse_type(source);

    match ty.kind {
        TypeKind::DynProtocol { bounds, bindings } => {
            assert_eq!(bounds.len(), 1, "Should have exactly 1 protocol bound");

            // Check the protocol bound
            match &bounds[0].kind {
                TypeBoundKind::Protocol(path) => {
                    assert_eq!(path.segments.len(), 1);
                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "Display");
                    } else {
                        panic!("Expected Name segment");
                    }
                }
                _ => panic!("Expected Protocol bound"),
            }

            // No bindings
            assert!(bindings.is_none(), "Should have no type bindings");
        }
        _ => panic!("Expected DynProtocol type, got: {:?}", ty.kind),
    }
}

#[test]
fn test_dyn_multiple_protocols() {
    let source = "dyn Display + Debug";
    let ty = parse_type(source);

    match ty.kind {
        TypeKind::DynProtocol { bounds, bindings } => {
            assert_eq!(bounds.len(), 2, "Should have exactly 2 protocol bounds");

            // Check Display bound
            match &bounds[0].kind {
                TypeBoundKind::Protocol(path) => {
                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "Display");
                    } else {
                        panic!("Expected Display protocol");
                    }
                }
                _ => panic!("Expected Protocol bound"),
            }

            // Check Debug bound
            match &bounds[1].kind {
                TypeBoundKind::Protocol(path) => {
                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "Debug");
                    } else {
                        panic!("Expected Debug protocol");
                    }
                }
                _ => panic!("Expected Protocol bound"),
            }

            assert!(bindings.is_none(), "Should have no type bindings");
        }
        _ => panic!("Expected DynProtocol type, got: {:?}", ty.kind),
    }
}

#[test]
fn test_dyn_with_single_type_binding() {
    let source = "dyn Iterator<Item = Int>";
    let ty = parse_type(source);

    match ty.kind {
        TypeKind::DynProtocol { bounds, bindings } => {
            assert_eq!(bounds.len(), 1, "Should have exactly 1 protocol bound");

            // Check Iterator bound
            match &bounds[0].kind {
                TypeBoundKind::Protocol(path) => {
                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "Iterator");
                    } else {
                        panic!("Expected Iterator protocol");
                    }
                }
                _ => panic!("Expected Protocol bound"),
            }

            // Check type bindings
            assert!(bindings.is_some(), "Should have type bindings");
            let bindings = bindings.unwrap();
            assert_eq!(bindings.len(), 1, "Should have 1 type binding");

            let binding = &bindings[0];
            assert_eq!(binding.name.name.as_str(), "Item");

            // Check that the bound type is Int
            match &binding.ty.kind {
                TypeKind::Int => {}
                _ => panic!(
                    "Expected Int type for Item binding, got: {:?}",
                    binding.ty.kind
                ),
            }
        }
        _ => panic!("Expected DynProtocol type, got: {:?}", ty.kind),
    }
}

#[test]
fn test_dyn_with_multiple_type_bindings() {
    let source = "dyn Iterator<Item = String, State = Int>";
    let ty = parse_type(source);

    match ty.kind {
        TypeKind::DynProtocol { bounds, bindings } => {
            assert_eq!(bounds.len(), 1, "Should have exactly 1 protocol bound");

            // Check type bindings
            assert!(bindings.is_some(), "Should have type bindings");
            let bindings = bindings.unwrap();
            assert_eq!(bindings.len(), 2, "Should have 2 type bindings");

            // Check Item = String
            let item_binding = &bindings[0];
            assert_eq!(item_binding.name.name.as_str(), "Item");
            match &item_binding.ty.kind {
                TypeKind::Path(path) => {
                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "String");
                    } else {
                        panic!("Expected String type");
                    }
                }
                _ => panic!("Expected Path type for Item binding"),
            }

            // Check State = Int
            let state_binding = &bindings[1];
            assert_eq!(state_binding.name.name.as_str(), "State");
            match &state_binding.ty.kind {
                TypeKind::Int => {}
                _ => panic!("Expected Int type for State binding"),
            }
        }
        _ => panic!("Expected DynProtocol type, got: {:?}", ty.kind),
    }
}

#[test]
fn test_dyn_with_bindings_and_additional_bounds() {
    let source = "dyn Iterator<Item = Int> + Display + Debug";
    let ty = parse_type(source);

    match ty.kind {
        TypeKind::DynProtocol { bounds, bindings } => {
            // Should have 3 bounds: Iterator, Display, Debug
            assert_eq!(bounds.len(), 3, "Should have exactly 3 protocol bounds");

            // Check Iterator bound
            match &bounds[0].kind {
                TypeBoundKind::Protocol(path) => {
                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "Iterator");
                    } else {
                        panic!("Expected Iterator protocol");
                    }
                }
                _ => panic!("Expected Protocol bound"),
            }

            // Check Display bound
            match &bounds[1].kind {
                TypeBoundKind::Protocol(path) => {
                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "Display");
                    } else {
                        panic!("Expected Display protocol");
                    }
                }
                _ => panic!("Expected Protocol bound"),
            }

            // Check Debug bound
            match &bounds[2].kind {
                TypeBoundKind::Protocol(path) => {
                    if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                        assert_eq!(ident.name.as_str(), "Debug");
                    } else {
                        panic!("Expected Debug protocol");
                    }
                }
                _ => panic!("Expected Protocol bound"),
            }

            // Check type bindings
            assert!(bindings.is_some(), "Should have type bindings");
            let bindings = bindings.unwrap();
            assert_eq!(bindings.len(), 1, "Should have 1 type binding");

            let binding = &bindings[0];
            assert_eq!(binding.name.name.as_str(), "Item");
            match &binding.ty.kind {
                TypeKind::Int => {}
                _ => panic!("Expected Int type for Item binding"),
            }
        }
        _ => panic!("Expected DynProtocol type, got: {:?}", ty.kind),
    }
}

#[test]
fn test_dyn_with_complex_type_binding() {
    let source = "dyn Container<Item = List<Int>>";
    let ty = parse_type(source);

    match ty.kind {
        TypeKind::DynProtocol { bounds, bindings } => {
            assert_eq!(bounds.len(), 1, "Should have exactly 1 protocol bound");

            // Check type bindings
            assert!(bindings.is_some(), "Should have type bindings");
            let bindings = bindings.unwrap();
            assert_eq!(bindings.len(), 1, "Should have 1 type binding");

            let binding = &bindings[0];
            assert_eq!(binding.name.name.as_str(), "Item");

            // Check that the bound type is List<Int>
            match &binding.ty.kind {
                TypeKind::Generic { base, args } => {
                    // Check base is List
                    match &base.kind {
                        TypeKind::Path(path) => {
                            if let verum_ast::PathSegment::Name(ident) = &path.segments[0] {
                                assert_eq!(ident.name.as_str(), "List");
                            } else {
                                panic!("Expected List type");
                            }
                        }
                        _ => panic!("Expected Path for List"),
                    }

                    // Check arg is Int
                    assert_eq!(args.len(), 1, "Should have 1 type argument");
                    match &args[0] {
                        GenericArg::Type(ty) => match &ty.kind {
                            TypeKind::Int => {}
                            _ => panic!("Expected Int type argument"),
                        },
                        _ => panic!("Expected Type generic argument"),
                    }
                }
                _ => panic!(
                    "Expected Generic type for Item binding, got: {:?}",
                    binding.ty.kind
                ),
            }
        }
        _ => panic!("Expected DynProtocol type, got: {:?}", ty.kind),
    }
}
