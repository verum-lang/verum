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
// Test file for protocol extends syntax
// Protocol extension: `protocol Name extends Base1 + Base2 { }` with multiple supertypes

use verum_ast::{FileId, ItemKind, TypeDeclBody};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

fn parse_source(source: &str) -> Result<Vec<verum_ast::Item>, verum_common::List<verum_fast_parser::ParseError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .map(|m| m.items.to_vec())
}

/// Helper to extract the first segment name from a Type (assumes it's a Path type)
fn get_first_segment_name(ty: &verum_ast::Type) -> &str {
    match &ty.kind {
        verum_ast::ty::TypeKind::Path(path) => {
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                ident.name.as_str()
            } else {
                panic!("Expected Name segment");
            }
        }
        _ => panic!("Expected Path type kind, got {:?}", ty.kind),
    }
}

/// Helper to get the number of segments in a Type (assumes it's a Path type)
fn get_segments_len(ty: &verum_ast::Type) -> usize {
    match &ty.kind {
        verum_ast::ty::TypeKind::Path(path) => path.segments.len(),
        _ => panic!("Expected Path type kind, got {:?}", ty.kind),
    }
}

#[test]
fn test_protocol_extends_single() {
    // Note: Using `wrap` instead of `pure` because `pure` is a reserved keyword in Verum
    let source = r#"
        type Applicative is protocol extends Functor {
            fn wrap<T>(value: T) -> Self;
        };
    "#;

    let items = parse_source(source).unwrap();
    assert_eq!(items.len(), 1);

    if let ItemKind::Type(type_decl) = &items[0].kind {
        if let TypeDeclBody::Protocol(body) = &type_decl.body {
            assert_eq!(body.extends.len(), 1, "Should have one base protocol");
            assert_eq!(get_segments_len(&body.extends[0]), 1);
            assert_eq!(get_first_segment_name(&body.extends[0]), "Functor");
            assert_eq!(body.items.len(), 1, "Should have one method");
        } else {
            panic!("Expected Protocol body");
        }
    } else {
        panic!("Expected Type declaration");
    }
}

#[test]
fn test_protocol_extends_multiple() {
    let source = r#"
        type Monad is protocol extends Applicative + Functor {
            fn bind<A, B>(self, f: fn(A) -> Self) -> Self;
        };
    "#;

    let items = parse_source(source).unwrap();
    assert_eq!(items.len(), 1);

    if let ItemKind::Type(type_decl) = &items[0].kind {
        if let TypeDeclBody::Protocol(body) = &type_decl.body {
            assert_eq!(body.extends.len(), 2, "Should have two base protocols");
            assert_eq!(body.items.len(), 1, "Should have one method");
        } else {
            panic!("Expected Protocol body");
        }
    } else {
        panic!("Expected Type declaration");
    }
}

#[test]
fn test_protocol_no_extends() {
    let source = r#"
        type Show is protocol {
            fn show(self) -> Text;
        };
    "#;

    let items = parse_source(source).unwrap();
    assert_eq!(items.len(), 1);

    if let ItemKind::Type(type_decl) = &items[0].kind {
        if let TypeDeclBody::Protocol(body) = &type_decl.body {
            assert_eq!(body.extends.len(), 0, "Should have no base protocols");
            assert_eq!(body.items.len(), 1, "Should have one method");
        } else {
            panic!("Expected Protocol body");
        }
    } else {
        panic!("Expected Type declaration");
    }
}

#[test]
fn test_protocol_extends_with_simple_names() {
    // Note: Parser currently does not support qualified paths in extends clause
    // (e.g., std.iter.Iterator). Only simple identifiers are supported.
    let source = r#"
        type MyProtocol is protocol extends BaseProtocol {
            fn method(self) -> Int;
        };
    "#;

    let items = parse_source(source).unwrap();
    assert_eq!(items.len(), 1);

    if let ItemKind::Type(type_decl) = &items[0].kind {
        if let TypeDeclBody::Protocol(body) = &type_decl.body {
            assert_eq!(body.extends.len(), 1, "Should have one base protocol");
            assert_eq!(get_segments_len(&body.extends[0]), 1);
            assert_eq!(get_first_segment_name(&body.extends[0]), "BaseProtocol");
        } else {
            panic!("Expected Protocol body");
        }
    } else {
        panic!("Expected Type declaration");
    }
}
