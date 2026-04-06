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
// Tests for affine type parsing
//
// Tests for Verum v6 syntax compliance Section 2.1
// Affine/linear types: at-most-once (affine) or exactly-once (linear) usage guarantees

use verum_ast::{ItemKind, ResourceModifier, TypeDecl, TypeDeclBody, span::FileId};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Helper to parse a type declaration from source code
fn parse_type_decl(source: &str) -> Result<TypeDecl, String> {
    let file_id = FileId::new(0);

    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    let module = parser
        .parse_module(lexer, file_id)
        .map_err(|e| format!("Parse error: {:?}", e))?;

    if module.items.len() != 1 {
        return Err(format!("Expected 1 item, found {}", module.items.len()));
    }

    match &module.items[0].kind {
        ItemKind::Type(decl) => Ok(decl.clone()),
        _ => Err("Not a type declaration".to_string()),
    }
}

#[test]
fn test_affine_type_declaration() {
    let source = "type affine FileHandle is { fd: Int };";
    let decl = parse_type_decl(source).expect("Failed to parse affine type");

    // Check that resource_modifier is set to Affine
    match decl.resource_modifier {
        Some(ResourceModifier::Affine) => {}
        _ => panic!("Expected Some(Affine), got {:?}", decl.resource_modifier),
    }

    // Check the type name
    assert_eq!(decl.name.name, "FileHandle");

    // Check that it's a record type
    assert!(matches!(decl.body, TypeDeclBody::Record(_)));
}

#[test]
fn test_linear_type_declaration() {
    let source = "type linear Handle is { fd: Int };";
    let decl = parse_type_decl(source).expect("Failed to parse linear type");

    // Check that resource_modifier is set to Linear
    match decl.resource_modifier {
        Some(ResourceModifier::Linear) => {}
        _ => panic!("Expected Some(Linear), got {:?}", decl.resource_modifier),
    }
}

#[test]
fn test_non_affine_type() {
    let source = "type Point is { x: Float, y: Float };";
    let decl = parse_type_decl(source).expect("Failed to parse normal type");

    // Check that resource_modifier is None
    match decl.resource_modifier {
        None => {}
        _ => panic!("Expected None, got {:?}", decl.resource_modifier),
    }
}

#[test]
fn test_affine_variant_type() {
    let source = "type affine Result<T, E> is Ok(T) | Err(E);";
    let decl = parse_type_decl(source).expect("Failed to parse affine variant");

    match decl.resource_modifier {
        Some(ResourceModifier::Affine) => {}
        _ => panic!("Expected Some(Affine), got {:?}", decl.resource_modifier),
    }

    assert!(matches!(decl.body, TypeDeclBody::Variant(_)));
}

#[test]
fn test_affine_alias_type() {
    let source = "type affine Handle is Int;";
    let decl = parse_type_decl(source).expect("Failed to parse affine alias");

    match decl.resource_modifier {
        Some(ResourceModifier::Affine) => {}
        _ => panic!("Expected Some(Affine), got {:?}", decl.resource_modifier),
    }

    assert!(matches!(decl.body, TypeDeclBody::Alias(_)));
}

#[test]
fn test_affine_with_generics() {
    let source = "type affine Container<T> is { value: T };";
    let decl = parse_type_decl(source).expect("Failed to parse affine generic type");

    match decl.resource_modifier {
        Some(ResourceModifier::Affine) => {}
        _ => panic!("Expected Some(Affine), got {:?}", decl.resource_modifier),
    }

    assert_eq!(decl.generics.len(), 1);
}

#[test]
fn test_affine_with_visibility() {
    let source = "pub type affine FileHandle is { fd: Int };";
    let decl = parse_type_decl(source).expect("Failed to parse public affine type");

    match decl.resource_modifier {
        Some(ResourceModifier::Affine) => {}
        _ => panic!("Expected Some(Affine), got {:?}", decl.resource_modifier),
    }

    assert!(decl.visibility.is_public());
}

#[test]
fn test_complex_affine_type() {
    let source = r#"
        type affine DatabaseConnection is {
            host: Text,
            port: Int,
            socket_fd: Int,
        };
    "#;

    let decl = parse_type_decl(source).expect("Failed to parse complex affine type");

    match decl.resource_modifier {
        Some(ResourceModifier::Affine) => {}
        _ => panic!("Expected Some(Affine), got {:?}", decl.resource_modifier),
    }

    match &decl.body {
        TypeDeclBody::Record(fields) => {
            assert_eq!(fields.len(), 3);
            assert_eq!(fields[0].name.name, "host");
            assert_eq!(fields[1].name.name, "port");
            assert_eq!(fields[2].name.name, "socket_fd");
        }
        _ => panic!("Expected record type"),
    }
}

#[test]
fn test_affine_tuple_type() {
    let source = "type affine Pair is (Int, Int);";
    let decl = parse_type_decl(source).expect("Failed to parse affine tuple");

    match decl.resource_modifier {
        Some(ResourceModifier::Affine) => {}
        _ => panic!("Expected Some(Affine), got {:?}", decl.resource_modifier),
    }

    assert!(matches!(decl.body, TypeDeclBody::Tuple(_)));
}

#[test]
fn test_multiple_type_declarations() {
    let source = r#"
        type Point is { x: Float, y: Float };
        type affine FileHandle is { fd: Int };
        type linear MustUse is { value: Int };
    "#;

    let file_id = FileId::new(0);

    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let module = parser
        .parse_module(lexer, file_id)
        .expect("Failed to parse module");

    assert_eq!(module.items.len(), 3);

    // First type: normal
    match &module.items[0].kind {
        ItemKind::Type(decl) => match decl.resource_modifier {
            None => {}
            _ => panic!("First type should not be affine"),
        },
        _ => panic!("Expected type declaration"),
    }

    // Second type: affine
    match &module.items[1].kind {
        ItemKind::Type(decl) => match decl.resource_modifier {
            Some(ResourceModifier::Affine) => {}
            _ => panic!("Second type should be affine"),
        },
        _ => panic!("Expected type declaration"),
    }

    // Third type: linear
    match &module.items[2].kind {
        ItemKind::Type(decl) => match decl.resource_modifier {
            Some(ResourceModifier::Linear) => {}
            _ => panic!("Third type should be linear"),
        },
        _ => panic!("Expected type declaration"),
    }
}
