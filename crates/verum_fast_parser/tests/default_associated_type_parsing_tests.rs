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
//! Tests for parsing default associated types in protocols
//!
//! Tests for default associated type syntax in protocol definitions

use verum_ast::decl::{ItemKind, ProtocolItemKind};
use verum_ast::span::FileId;
use verum_ast::ty::TypeKind;
use verum_common::List;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse(source: &str) -> Result<List<verum_ast::Item>, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .map(|module| module.items)
        .map_err(|e| format!("{:?}", e))
}

#[test]
fn test_default_associated_type_simple() {
    // Test: protocol Container { default type Item = Heap<u8>; }
    let source = r#"
        protocol Container {
            default type Item = Heap<u8>;
            fn get(self: &Self, idx: usize) -> Self.Item;
        }
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    assert_eq!(items.len(), 1);

    match &items[0].kind {
        ItemKind::Protocol(protocol_decl) => {
            assert_eq!(protocol_decl.name.name.as_str(), "Container");
            assert_eq!(protocol_decl.items.len(), 2); // Item type + get function

            // Check the first item is a Type with default
            match &protocol_decl.items[0].kind {
                ProtocolItemKind::Type {
                    name,
                    type_params,
                    bounds,
                    where_clause,
                    default_type,
                } => {
                    assert_eq!(name.name.as_str(), "Item");
                    assert!(type_params.is_empty());
                    assert!(bounds.is_empty());
                    assert!(where_clause.is_none());

                    // Check default type is Some
                    assert!(
                        default_type.is_some(),
                        "Expected default type to be present"
                    );

                    let default_ty = default_type.as_ref().unwrap();
                    // Should be Heap<u8>
                    match &default_ty.kind {
                        TypeKind::Generic { base, args } => {
                            // Base should be Heap
                            match &base.kind {
                                TypeKind::Path(path) => {
                                    assert_eq!(path.as_ident().unwrap().as_str(), "Heap");
                                }
                                _ => panic!("Expected Path for Heap, got: {:?}", base.kind),
                            }
                            // Args should contain u8
                            assert_eq!(args.len(), 1);
                        }
                        _ => panic!("Expected Generic type, got: {:?}", default_ty.kind),
                    }
                }
                _ => panic!(
                    "Expected Type protocol item, got: {:?}",
                    protocol_decl.items[0].kind
                ),
            }
        }
        _ => panic!("Expected Protocol item, got: {:?}", items[0].kind),
    }
}

#[test]
fn test_default_associated_type_without_keyword() {
    // Test: protocol Container { type Item = Heap<u8>; }
    // This should also work - 'default' keyword is optional for backwards compatibility
    let source = r#"
        protocol Container {
            type Item = Heap<u8>;
        }
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Protocol(protocol_decl) => match &protocol_decl.items[0].kind {
            ProtocolItemKind::Type { default_type, .. } => {
                assert!(
                    default_type.is_some(),
                    "Expected default type to be present"
                );
            }
            _ => panic!("Expected Type protocol item"),
        },
        _ => panic!("Expected Protocol item"),
    }
}

#[test]
fn test_associated_type_no_default() {
    // Test: protocol Container { type Item; }
    // No default, should parse as Maybe::None
    let source = r#"
        protocol Container {
            type Item;
        }
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Protocol(protocol_decl) => match &protocol_decl.items[0].kind {
            ProtocolItemKind::Type { default_type, .. } => {
                assert!(default_type.is_none(), "Expected no default type");
            }
            _ => panic!("Expected Type protocol item"),
        },
        _ => panic!("Expected Protocol item"),
    }
}

#[test]
fn test_default_associated_type_with_bounds() {
    // Test: protocol Container { default type Item: Clone = Int; }
    let source = r#"
        protocol Container {
            default type Item: Clone = Int;
        }
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Protocol(protocol_decl) => {
            match &protocol_decl.items[0].kind {
                ProtocolItemKind::Type {
                    name,
                    bounds,
                    default_type,
                    ..
                } => {
                    assert_eq!(name.name.as_str(), "Item");
                    assert_eq!(bounds.len(), 1);
                    assert_eq!(bounds[0].as_ident().unwrap().as_str(), "Clone");
                    assert!(default_type.is_some());

                    let default_ty = default_type.as_ref().unwrap();
                    match &default_ty.kind {
                        TypeKind::Int => {} // Int is correct
                        _ => panic!("Expected Int type, got: {:?}", default_ty.kind),
                    }
                }
                _ => panic!("Expected Type protocol item"),
            }
        }
        _ => panic!("Expected Protocol item"),
    }
}

#[test]
fn test_default_keyword_without_value_error() {
    // Test: protocol Container { default type Item; }
    // Should error because 'default' keyword requires '= Type'
    let source = r#"
        protocol Container {
            default type Item;
        }
    "#;

    let result = parse(source);
    // This should error or the parser should reject it
    assert!(
        result.is_err(),
        "Expected error for 'default' keyword without value"
    );
}

#[test]
fn test_multiple_associated_types_with_defaults() {
    // Test multiple associated types, some with defaults
    let source = r#"
        protocol Container {
            type Key;
            default type Value = Text;
            default type Error = Text;
        }
    "#;

    let result = parse(source);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let items = result.unwrap();
    match &items[0].kind {
        ItemKind::Protocol(protocol_decl) => {
            assert_eq!(protocol_decl.items.len(), 3);

            // First: Key (no default)
            match &protocol_decl.items[0].kind {
                ProtocolItemKind::Type {
                    name, default_type, ..
                } => {
                    assert_eq!(name.name.as_str(), "Key");
                    assert!(default_type.is_none());
                }
                _ => panic!("Expected Type protocol item"),
            }

            // Second: Value (with default)
            match &protocol_decl.items[1].kind {
                ProtocolItemKind::Type {
                    name, default_type, ..
                } => {
                    assert_eq!(name.name.as_str(), "Value");
                    assert!(default_type.is_some());
                }
                _ => panic!("Expected Type protocol item"),
            }

            // Third: Error (with default)
            match &protocol_decl.items[2].kind {
                ProtocolItemKind::Type {
                    name, default_type, ..
                } => {
                    assert_eq!(name.name.as_str(), "Error");
                    assert!(default_type.is_some());
                }
                _ => panic!("Expected Type protocol item"),
            }
        }
        _ => panic!("Expected Protocol item"),
    }
}
