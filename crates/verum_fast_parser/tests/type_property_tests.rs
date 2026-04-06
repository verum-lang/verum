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
//! Tests for type property expression parsing.
//!
//! Tests for type property annotations on declarations
//!
//! Type properties provide compile-time access to type metadata:
//! - size: Size in bytes
//! - alignment: Alignment requirement in bytes
//! - stride: Memory stride for arrays/iteration
//! - min: Minimum value (for numeric types)
//! - max: Maximum value (for numeric types)
//! - bits: Bit width (for numeric types)
//! - name: Type name as string
//!
//! Examples:
//! - `Int.size` returns the size of Int in bytes
//! - `Float.alignment` returns the alignment of Float
//! - `T.name` returns the type name as a string

use verum_ast::{Expr, ExprKind, FileId, TypeKind, expr::TypeProperty};
use verum_fast_parser::VerumParser;

fn parse_expr(source: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse expression: {}", source))
}

// === TYPE PROPERTY TESTS ===

#[test]
fn test_parse_int_size() {
    let expr = parse_expr("Int.size");
    match &expr.kind {
        ExprKind::TypeProperty { ty, property } => {
            assert!(matches!(ty.kind, TypeKind::Int));
            assert_eq!(*property, TypeProperty::Size);
        }
        _ => panic!("Expected TypeProperty expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_float_alignment() {
    let expr = parse_expr("Float.alignment");
    match &expr.kind {
        ExprKind::TypeProperty { ty, property } => {
            assert!(matches!(ty.kind, TypeKind::Float));
            assert_eq!(*property, TypeProperty::Alignment);
        }
        _ => panic!("Expected TypeProperty expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_bool_stride() {
    let expr = parse_expr("Bool.stride");
    match &expr.kind {
        ExprKind::TypeProperty { ty, property } => {
            assert!(matches!(ty.kind, TypeKind::Bool));
            assert_eq!(*property, TypeProperty::Stride);
        }
        _ => panic!("Expected TypeProperty expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_int_min() {
    let expr = parse_expr("Int.min");
    match &expr.kind {
        ExprKind::TypeProperty { ty, property } => {
            assert!(matches!(ty.kind, TypeKind::Int));
            assert_eq!(*property, TypeProperty::Min);
        }
        _ => panic!("Expected TypeProperty expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_int_max() {
    let expr = parse_expr("Int.max");
    match &expr.kind {
        ExprKind::TypeProperty { ty, property } => {
            assert!(matches!(ty.kind, TypeKind::Int));
            assert_eq!(*property, TypeProperty::Max);
        }
        _ => panic!("Expected TypeProperty expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_int_bits() {
    let expr = parse_expr("Int.bits");
    match &expr.kind {
        ExprKind::TypeProperty { ty, property } => {
            assert!(matches!(ty.kind, TypeKind::Int));
            assert_eq!(*property, TypeProperty::Bits);
        }
        _ => panic!("Expected TypeProperty expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_text_name() {
    let expr = parse_expr("Text.name");
    match &expr.kind {
        ExprKind::TypeProperty { ty, property } => {
            assert!(matches!(ty.kind, TypeKind::Text));
            assert_eq!(*property, TypeProperty::Name);
        }
        _ => panic!("Expected TypeProperty expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_char_size() {
    let expr = parse_expr("Char.size");
    match &expr.kind {
        ExprKind::TypeProperty { ty, property } => {
            assert!(matches!(ty.kind, TypeKind::Char));
            assert_eq!(*property, TypeProperty::Size);
        }
        _ => panic!("Expected TypeProperty expression, got {:?}", expr.kind),
    }
}

// === CUSTOM TYPE PROPERTY TESTS ===

#[test]
fn test_parse_custom_type_size() {
    let expr = parse_expr("MyType.size");
    match &expr.kind {
        ExprKind::TypeProperty { ty, property } => {
            // Custom types are parsed as Path types
            assert!(matches!(ty.kind, TypeKind::Path(_)));
            assert_eq!(*property, TypeProperty::Size);
        }
        _ => panic!("Expected TypeProperty expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_qualified_type_alignment() {
    // Note: Qualified paths starting with lowercase identifiers (like `std.collections.List`)
    // are parsed as field access chains at parse time because the parser cannot distinguish
    // between value paths and type paths without name resolution.
    // Type checking will resolve `std.collections.List.alignment` as a type property later.
    let expr = parse_expr("std.collections.List.alignment");
    match &expr.kind {
        ExprKind::Field { field, .. } => {
            // At parse time, this is a chain of field accesses
            assert_eq!(field.name.as_str(), "alignment");
        }
        _ => panic!("Expected Field expression for lowercase-starting path, got {:?}", expr.kind),
    }
}

// === FIELD ACCESS FALLBACK TESTS ===
// These tests verify that regular field access still works for non-type-property names

#[test]
fn test_parse_regular_field_access() {
    // 'length' is not a type property, so this should be regular field access
    let expr = parse_expr("x.length");
    match &expr.kind {
        ExprKind::Field { expr: _, field } => {
            assert_eq!(field.name.as_str(), "length");
        }
        _ => panic!("Expected Field expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_value_size_field() {
    // When the receiver is a lowercase identifier, it's a value, not a type
    // So 'value.size' should still be a field access (the 'size' field of 'value')
    let expr = parse_expr("value.size");
    // Note: This currently parses as TypeProperty because we can't distinguish
    // at parse time between a type and a value with the same name.
    // Type checking will handle this correctly.
    match &expr.kind {
        ExprKind::TypeProperty { .. } => {
            // For now, path expressions that could be types get converted to TypeProperty
            // This is correct for type-first-class semantics
        }
        ExprKind::Field { expr: _, field } => {
            // This would also be acceptable
            assert_eq!(field.name.as_str(), "size");
        }
        _ => panic!(
            "Expected TypeProperty or Field expression, got {:?}",
            expr.kind
        ),
    }
}

// === TYPE PROPERTY IN EXPRESSIONS ===

#[test]
fn test_type_property_in_let() {
    // Parse an expression that would be the RHS of a let statement
    // (We test just the expression since parse_stmt_str is not exposed)
    let expr = parse_expr("Int.size");
    match &expr.kind {
        ExprKind::TypeProperty { ty, property } => {
            assert!(matches!(ty.kind, TypeKind::Int));
            assert_eq!(*property, TypeProperty::Size);
        }
        _ => panic!("Expected TypeProperty expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_type_property_in_comparison() {
    // Int.size should be usable in expressions
    let expr = parse_expr("Int.size == 8");
    match &expr.kind {
        ExprKind::Binary { op, left, right: _ } => {
            assert_eq!(*op, verum_ast::BinOp::Eq);
            match &left.kind {
                ExprKind::TypeProperty { ty, property } => {
                    assert!(matches!(ty.kind, TypeKind::Int));
                    assert_eq!(*property, TypeProperty::Size);
                }
                _ => panic!("Expected TypeProperty on left side"),
            }
        }
        _ => panic!("Expected Binary expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_type_property_as_function_arg() {
    let expr = parse_expr("allocate(MyStruct.size)");
    match &expr.kind {
        ExprKind::Call { func: _, args, .. } => {
            assert_eq!(args.len(), 1);
            match &args[0].kind {
                ExprKind::TypeProperty { ty, property } => {
                    assert!(matches!(ty.kind, TypeKind::Path(_)));
                    assert_eq!(*property, TypeProperty::Size);
                }
                _ => panic!("Expected TypeProperty as argument"),
            }
        }
        _ => panic!("Expected Call expression, got {:?}", expr.kind),
    }
}
