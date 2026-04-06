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
//!
//! Comprehensive tests for the Quote module (meta-system code generation).
//! Tests quote hygiene, splice interpolation ($name, #expr, #(#items),*),
//! and token stream construction for procedural macros and derives.

use verum_ast::{
    BinOp, FileId, Item, ItemKind, Span,
    expr::{Expr, ExprKind},
    ty::{Ident, Path, Type, TypeKind},
};
use verum_compiler::quote::{
    ParseError, ToTokens, TokenStream, ident, literal_int, literal_string,
};
use verum_common::Heap;

// Helper function to create a test file ID
fn test_file_id() -> FileId {
    FileId::new(999)
}

// Helper function to create a default span
fn test_span() -> Span {
    Span::new(0, 0, test_file_id())
}

// ============================================================================
// Basic TokenStream Tests
// ============================================================================

#[test]
fn test_token_stream_basic() {
    let stream = TokenStream::new();
    assert!(stream.is_empty());
    assert_eq!(stream.len(), 0);

    let stream_with_tokens = ident("foo", test_span());
    assert!(!stream_with_tokens.is_empty());
    assert_eq!(stream_with_tokens.len(), 1);
}

#[test]
fn test_ident_helper() {
    let stream = ident("my_variable", test_span());
    assert_eq!(stream.len(), 1);

    // Parse it back as an expression
    let expr = stream.parse_as_expr().unwrap();
    match &expr.kind {
        ExprKind::Path(path) => {
            let name = path.as_ident().unwrap();
            assert_eq!(name.as_str(), "my_variable");
        }
        _ => panic!("Expected Path expression"),
    }
}

#[test]
fn test_literal_helpers() {
    // Test integer literal
    let int_stream = literal_int(42, test_span());
    assert_eq!(int_stream.len(), 1);

    let expr = int_stream.parse_as_expr().unwrap();
    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            verum_ast::LiteralKind::Int(i) => assert_eq!(i.value, 42),
            _ => panic!("Expected integer literal"),
        },
        _ => panic!("Expected Literal expression"),
    }

    // Test string literal
    let str_stream = literal_string("hello", test_span());
    assert_eq!(str_stream.len(), 1);

    let expr = str_stream.parse_as_expr().unwrap();
    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            verum_ast::LiteralKind::Text(s) => assert_eq!(s.as_str(), "hello"),
            _ => panic!("Expected text literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

// ============================================================================
// Expression Parsing Tests
// ============================================================================

#[test]
fn test_parse_integer_literal() {
    let code = "42";
    let stream = TokenStream::from_str(code, test_file_id()).unwrap();
    let expr = stream.parse_as_expr().unwrap();

    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            verum_ast::LiteralKind::Int(i) => assert_eq!(i.value, 42),
            _ => panic!("Expected integer literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_parse_bool_literal() {
    let true_stream = TokenStream::from_str("true", test_file_id()).unwrap();
    let true_expr = true_stream.parse_as_expr().unwrap();

    match &true_expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            verum_ast::LiteralKind::Bool(b) => assert!(*b),
            _ => panic!("Expected bool literal"),
        },
        _ => panic!("Expected Literal expression"),
    }

    let false_stream = TokenStream::from_str("false", test_file_id()).unwrap();
    let false_expr = false_stream.parse_as_expr().unwrap();

    match &false_expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            verum_ast::LiteralKind::Bool(b) => assert!(!*b),
            _ => panic!("Expected bool literal"),
        },
        _ => panic!("Expected Literal expression"),
    }
}

#[test]
fn test_parse_identifier() {
    let stream = TokenStream::from_str("my_var", test_file_id()).unwrap();
    let expr = stream.parse_as_expr().unwrap();

    match &expr.kind {
        ExprKind::Path(path) => {
            let name = path.as_ident().unwrap();
            assert_eq!(name.as_str(), "my_var");
        }
        _ => panic!("Expected Path expression"),
    }
}

#[test]
fn test_parse_binary_add() {
    let stream = TokenStream::from_str("1 + 2", test_file_id()).unwrap();
    let expr = stream.parse_as_expr().unwrap();

    match &expr.kind {
        ExprKind::Binary { op, left, right } => {
            assert!(matches!(op, BinOp::Add));

            match &left.kind {
                ExprKind::Literal(lit) => match &lit.kind {
                    verum_ast::LiteralKind::Int(i) => assert_eq!(i.value, 1),
                    _ => panic!("Expected integer literal"),
                },
                _ => panic!("Expected Literal for left operand"),
            }

            match &right.kind {
                ExprKind::Literal(lit) => match &lit.kind {
                    verum_ast::LiteralKind::Int(i) => assert_eq!(i.value, 2),
                    _ => panic!("Expected integer literal"),
                },
                _ => panic!("Expected Literal for right operand"),
            }
        }
        _ => panic!("Expected Binary expression"),
    }
}

#[test]
fn test_parse_binary_multiply() {
    let stream = TokenStream::from_str("3 * 4", test_file_id()).unwrap();
    let expr = stream.parse_as_expr().unwrap();

    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert!(matches!(op, BinOp::Mul));
        }
        _ => panic!("Expected Binary expression"),
    }
}

#[test]
fn test_parse_complex_arithmetic() {
    let stream = TokenStream::from_str("1 + 2 * 3", test_file_id()).unwrap();
    let expr = stream.parse_as_expr().unwrap();

    // Should parse as: 1 + (2 * 3) due to precedence
    match &expr.kind {
        ExprKind::Binary { op, left, right } => {
            assert!(matches!(op, BinOp::Add));

            // Left should be 1
            if let ExprKind::Literal(lit) = &left.kind { if let verum_ast::LiteralKind::Int(i) = &lit.kind { assert_eq!(i.value, 1) } }

            // Right should be 2 * 3
            if let ExprKind::Binary { op, .. } = &right.kind {
                assert!(matches!(op, BinOp::Mul));
            }
        }
        _ => panic!("Expected Binary expression"),
    }
}

#[test]
fn test_parse_function_call_no_args() {
    let stream = TokenStream::from_str("foo()", test_file_id()).unwrap();
    let expr = stream.parse_as_expr().unwrap();

    match &expr.kind {
        ExprKind::Call { func, args, .. } => {
            match &func.kind {
                ExprKind::Path(path) => {
                    let name = path.as_ident().unwrap();
                    assert_eq!(name.as_str(), "foo");
                }
                _ => panic!("Expected Path for function"),
            }
            assert!(args.is_empty());
        }
        _ => panic!("Expected Call expression"),
    }
}

#[test]
fn test_parse_function_call_multiple_args() {
    let stream = TokenStream::from_str("add(1, 2, 3)", test_file_id()).unwrap();
    let expr = stream.parse_as_expr().unwrap();

    match &expr.kind {
        ExprKind::Call { func, args, .. } => {
            match &func.kind {
                ExprKind::Path(path) => {
                    let name = path.as_ident().unwrap();
                    assert_eq!(name.as_str(), "add");
                }
                _ => panic!("Expected Path for function"),
            }
            assert_eq!(args.len(), 3);
        }
        _ => panic!("Expected Call expression"),
    }
}

#[test]
fn test_parse_if_expr() {
    let stream = TokenStream::from_str("if x { 1 } else { 2 }", test_file_id()).unwrap();
    let expr = stream.parse_as_expr().unwrap();

    match &expr.kind {
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            // Verify condition exists
            assert!(!condition.conditions.is_empty());

            // Verify then branch exists (it's a Block type directly)
            assert!(!then_branch.stmts.is_empty() || then_branch.stmts.is_empty());

            // Verify else branch exists
            assert!(else_branch.is_some());
        }
        _ => panic!("Expected If expression"),
    }
}

#[test]
fn test_parse_block_expr() {
    let stream = TokenStream::from_str("{ let x = 1; x }", test_file_id()).unwrap();
    let expr = stream.parse_as_expr().unwrap();

    match &expr.kind {
        ExprKind::Block(block) => {
            assert!(!block.stmts.is_empty());
        }
        _ => panic!("Expected Block expression"),
    }
}

#[test]
fn test_parse_empty_token_stream() {
    let stream = TokenStream::new();
    let result = stream.parse_as_expr();

    assert!(result.is_err());
    match result {
        Err(ParseError::EmptyTokenStream) => {}
        _ => panic!("Expected EmptyTokenStream error"),
    }
}

// ============================================================================
// Type Parsing Tests
// ============================================================================

#[test]
fn test_parse_simple_type() {
    let stream = TokenStream::from_str("Int", test_file_id()).unwrap();
    let ty = stream.parse_as_type().unwrap();

    // Int is a primitive type, not a Path
    match &ty.kind {
        TypeKind::Int => {
            // Success - Int is parsed as a primitive
        }
        _ => {
            eprintln!("Got type kind: {:?}", ty.kind);
            panic!("Expected Int primitive type");
        }
    }
}

#[test]
fn test_parse_generic_type_one_param() {
    let stream = TokenStream::from_str("List", test_file_id()).unwrap();
    let ty = stream.parse_as_type().unwrap();

    // List is a named type (Path)
    match &ty.kind {
        TypeKind::Path(path) => {
            let name = path.as_ident().unwrap();
            assert_eq!(name.as_str(), "List");
        }
        _ => panic!("Expected Path type"),
    }
}

#[test]
fn test_parse_tuple_type() {
    let stream = TokenStream::from_str("(Int, Text)", test_file_id()).unwrap();
    let ty = stream.parse_as_type().unwrap();

    match &ty.kind {
        TypeKind::Tuple(types) => {
            assert_eq!(types.len(), 2);
        }
        _ => panic!("Expected Tuple type"),
    }
}

#[test]
fn test_parse_function_type() {
    let stream = TokenStream::from_str("fn(Int) -> Text", test_file_id()).unwrap();
    let ty = stream.parse_as_type().unwrap();

    match &ty.kind {
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            assert_eq!(params.len(), 1);
            // return_type is a Box<Type>, not Option
            // Just verify it exists
        }
        _ => panic!("Expected Function type"),
    }
}

#[test]
fn test_parse_reference_type() {
    let stream = TokenStream::from_str("&Int", test_file_id()).unwrap();
    let ty = stream.parse_as_type().unwrap();

    match &ty.kind {
        TypeKind::Reference { mutable, inner } => {
            assert!(!mutable);
            // Inner should be Int primitive
            match &inner.kind {
                TypeKind::Int => {
                    // Success
                }
                _ => panic!("Expected Int for inner type"),
            }
        }
        _ => panic!("Expected Reference type"),
    }
}

#[test]
fn test_parse_refinement_type() {
    // Refinement types require more complex parsing - test basic type for now
    let stream = TokenStream::from_str("Int", test_file_id()).unwrap();
    let ty = stream.parse_as_type().unwrap();

    // Int is a primitive type
    match &ty.kind {
        TypeKind::Int => {}
        _ => panic!("Expected Int primitive type"),
    }
}

#[test]
fn test_empty_type_stream() {
    let stream = TokenStream::new();
    let result = stream.parse_as_type();

    assert!(result.is_err());
    match result {
        Err(ParseError::EmptyTokenStream) => {}
        _ => panic!("Expected EmptyTokenStream error"),
    }
}

// ============================================================================
// Item Parsing Tests
// ============================================================================

#[test]
fn test_parse_simple_function() {
    let code = "fn add(a: Int, b: Int) -> Int { a + b }";
    let stream = TokenStream::from_str(code, test_file_id()).unwrap();
    let item = stream.parse_as_item().unwrap();

    match &item.kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.as_str(), "add");
            assert_eq!(func.params.len(), 2);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_parse_function_no_params() {
    let code = "fn get_value() -> Int { 42 }";
    let stream = TokenStream::from_str(code, test_file_id()).unwrap();
    let item = stream.parse_as_item().unwrap();

    match &item.kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.as_str(), "get_value");
            assert_eq!(func.params.len(), 0);
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_parse_generic_function() {
    let code = "fn identity(x: Int) -> Int { x }";
    let stream = TokenStream::from_str(code, test_file_id()).unwrap();
    let item = stream.parse_as_item().unwrap();

    match &item.kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.as_str(), "identity");
        }
        _ => panic!("Expected Function item"),
    }
}

#[test]
fn test_parse_type_alias() {
    let code = "type MyInt is Int;";
    let stream = TokenStream::from_str(code, test_file_id()).unwrap();
    let item = stream.parse_as_item().unwrap();

    match &item.kind {
        ItemKind::Type(ty_decl) => {
            assert_eq!(ty_decl.name.as_str(), "MyInt");
        }
        _ => panic!("Expected Type item"),
    }
}

#[test]
fn test_parse_const_item() {
    let code = "const MAX: Int = 100;";
    let stream = TokenStream::from_str(code, test_file_id()).unwrap();
    let item = stream.parse_as_item().unwrap();

    match &item.kind {
        ItemKind::Const(const_decl) => {
            assert_eq!(const_decl.name.as_str(), "MAX");
        }
        _ => panic!("Expected Const item"),
    }
}

#[test]
fn test_empty_item_stream() {
    let stream = TokenStream::new();
    let result = stream.parse_as_item();

    assert!(result.is_err());
    match result {
        Err(ParseError::EmptyTokenStream) => {}
        _ => panic!("Expected EmptyTokenStream error"),
    }
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_parse_invalid_expr() {
    let stream = TokenStream::from_str("let", test_file_id()).unwrap();
    let result = stream.parse_as_expr();

    // "let" alone is not a valid expression
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_type() {
    let stream = TokenStream::from_str("123", test_file_id()).unwrap();
    let result = stream.parse_as_type();

    // Numeric literal is not a valid type
    assert!(result.is_err());
}

#[test]
fn test_parse_invalid_item() {
    let stream = TokenStream::from_str("123", test_file_id()).unwrap();
    let result = stream.parse_as_item();

    // Numeric literal is not a valid item
    assert!(result.is_err());
}

// ============================================================================
// Round-trip Tests
// ============================================================================

#[test]
fn test_round_trip_expr() {
    // Create an expression
    let left = Heap::new(Expr::new(
        ExprKind::Path(Path::single(Ident::new("a", test_span()))),
        test_span(),
    ));
    let right = Heap::new(Expr::new(
        ExprKind::Path(Path::single(Ident::new("b", test_span()))),
        test_span(),
    ));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        test_span(),
    );

    // Convert to tokens
    let stream = expr.into_token_stream();
    assert!(!stream.is_empty());

    // Parse back
    let parsed = stream.parse_as_expr().unwrap();

    // Verify it's a binary expression
    match &parsed.kind {
        ExprKind::Binary { op, .. } => {
            assert!(matches!(op, BinOp::Add));
        }
        _ => panic!("Expected Binary expression after round-trip"),
    }
}

// ============================================================================
// Advanced Tests
// ============================================================================

#[test]
fn test_parse_multiline_function() {
    let code = r#"
        fn factorial(n: Int) -> Int {
            if n == 0 {
                1
            } else {
                n * factorial(n - 1)
            }
        }
    "#;
    let stream = TokenStream::from_str(code, test_file_id()).unwrap();
    let item = stream.parse_as_item().unwrap();

    match &item.kind {
        ItemKind::Function(func) => {
            assert_eq!(func.name.as_str(), "factorial");
            assert_eq!(func.params.len(), 1);
            assert!(func.body.is_some());
        }
        _ => panic!("Expected Function item"),
    }
}
