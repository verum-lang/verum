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
//! Tests that const expressions are properly evaluated during MIR lowering phase.
//! This includes array sizes, repeat counts, and compile-time constant expressions.

use verum_ast::{
    Ident, Module, Type,
    decl::{FunctionBody, FunctionDecl, Visibility},
    expr::{ArrayExpr, Expr, ExprKind},
    literal::{IntLit, Literal, LiteralKind},
    span::Span,
    ty::{Path, PathSegment, TypeKind},
};
use verum_compiler::phases::mir_lowering::{LoweringContext, MirType};
use verum_common::{List, Maybe};

/// Helper to create a module with a simple function
fn create_test_module(body_expr: Expr) -> Module {
    let func = FunctionDecl {
        visibility: Visibility::Private,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new("test_func", Span::dummy()),
        generics: List::new(),
        params: List::new(),
        throws_clause: Maybe::None,
        return_type: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(FunctionBody::Expr(body_expr)),
        span: Span::dummy(),
    };

    Module {
        items: vec![verum_ast::Item {
            kind: verum_ast::decl::ItemKind::Function(func),
            attributes: List::new(),
            span: Span::dummy(),
        }].into(),
        attributes: List::new(),
        file_id: verum_ast::FileId::dummy(),
        span: Span::dummy(),
    }
}

/// Helper to create an integer literal expression
fn int_lit(value: i128) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value,
                suffix: None,
            }),
            span: Span::dummy(),
        }),
        Span::dummy(),
    )
}

/// Test 1: Array type with const size evaluation
#[test]
fn test_array_size_const_eval() {
    let mut ctx = LoweringContext::new();

    // Create array type: [i32; 5]
    let array_type = Type {
        kind: TypeKind::Array {
            element: Box::new(Type {
                kind: TypeKind::Int,
                span: Span::dummy(),
            }),
            size: Some(Box::new(int_lit(5))),
        },
        span: Span::dummy(),
    };

    let mir_type = ctx.lower_type(&array_type);

    // Verify the array size was evaluated
    match mir_type {
        MirType::Array(_elem, size) => {
            assert_eq!(size, 5, "Array size should be 5");
        }
        _ => panic!("Expected MirType::Array, got {:?}", mir_type),
    }
}

/// Test 2: Array type with computed const size
#[test]
fn test_array_size_computed_const() {
    let mut ctx = LoweringContext::new();

    // Create array type: [i32; 2 + 3]
    let size_expr = Expr::new(
        ExprKind::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: Box::new(int_lit(2)),
            right: Box::new(int_lit(3)),
        },
        Span::dummy(),
    );

    let array_type = Type {
        kind: TypeKind::Array {
            element: Box::new(Type {
                kind: TypeKind::Int,
                span: Span::dummy(),
            }),
            size: Some(Box::new(size_expr)),
        },
        span: Span::dummy(),
    };

    let mir_type = ctx.lower_type(&array_type);

    // Verify the computed size
    match mir_type {
        MirType::Array(_elem, size) => {
            assert_eq!(size, 5, "Array size should be 2 + 3 = 5");
        }
        _ => panic!("Expected MirType::Array, got {:?}", mir_type),
    }
}

/// Test 3: Array repeat expression with const count
#[test]
fn test_array_repeat_const_eval() {
    let mut ctx = LoweringContext::new();

    // Create expression: [0; 10]
    let repeat_expr = Expr::new(
        ExprKind::Array(ArrayExpr::Repeat {
            value: Box::new(int_lit(0)),
            count: Box::new(int_lit(10)),
        }),
        Span::dummy(),
    );

    let module = create_test_module(repeat_expr);

    // Lower the module - should not panic
    let result = ctx.lower_module(&module);

    // Verify no diagnostics were generated for const evaluation
    assert!(result.is_ok(), "Module lowering should succeed");
}

/// Test 4: Array repeat with computed count
#[test]
fn test_array_repeat_computed_count() {
    let mut ctx = LoweringContext::new();

    // Create expression: [42; 3 * 4]
    let count_expr = Expr::new(
        ExprKind::Binary {
            op: verum_ast::expr::BinOp::Mul,
            left: Box::new(int_lit(3)),
            right: Box::new(int_lit(4)),
        },
        Span::dummy(),
    );

    let repeat_expr = Expr::new(
        ExprKind::Array(ArrayExpr::Repeat {
            value: Box::new(int_lit(42)),
            count: Box::new(count_expr),
        }),
        Span::dummy(),
    );

    let module = create_test_module(repeat_expr);

    // Lower the module - should evaluate 3 * 4 = 12
    let result = ctx.lower_module(&module);
    assert!(result.is_ok(), "Module lowering should succeed");
}

/// Test 5: Nested array with const sizes
#[test]
fn test_nested_array_const_sizes() {
    let mut ctx = LoweringContext::new();

    // Create type: [[i32; 3]; 2]
    let inner_array = Type {
        kind: TypeKind::Array {
            element: Box::new(Type {
                kind: TypeKind::Int,
                span: Span::dummy(),
            }),
            size: Some(Box::new(int_lit(3))),
        },
        span: Span::dummy(),
    };

    let outer_array = Type {
        kind: TypeKind::Array {
            element: Box::new(inner_array),
            size: Some(Box::new(int_lit(2))),
        },
        span: Span::dummy(),
    };

    let mir_type = ctx.lower_type(&outer_array);

    // Verify nested array sizes
    match mir_type {
        MirType::Array(elem, outer_size) => {
            assert_eq!(outer_size, 2, "Outer array size should be 2");
            match *elem {
                MirType::Array(_, inner_size) => {
                    assert_eq!(inner_size, 3, "Inner array size should be 3");
                }
                _ => panic!("Expected inner MirType::Array"),
            }
        }
        _ => panic!("Expected MirType::Array, got {:?}", mir_type),
    }
}

/// Test 6: Error handling for non-constant array size
#[test]
fn test_array_size_non_const_error() {
    let mut ctx = LoweringContext::new();

    // Create expression that references a variable (non-const)
    let non_const_expr = Expr::new(
        ExprKind::Path(Path::new(
            vec![PathSegment::Name(Ident::new("n", Span::dummy()))].into(),
            Span::dummy(),
        )),
        Span::dummy(),
    );

    let array_type = Type {
        kind: TypeKind::Array {
            element: Box::new(Type {
                kind: TypeKind::Int,
                span: Span::dummy(),
            }),
            size: Some(Box::new(non_const_expr)),
        },
        span: Span::dummy(),
    };

    let _mir_type = ctx.lower_type(&array_type);

    // Should have generated a diagnostic for failed evaluation
    assert!(
        !ctx.diagnostics.is_empty(),
        "Should have diagnostic for non-const array size"
    );
}

/// Test 7: Complex const expression with multiple operations
#[test]
fn test_complex_const_expression() {
    let mut ctx = LoweringContext::new();

    // Create expression: [(2 + 3) * 4 - 1; 10]
    let add_expr = Expr::new(
        ExprKind::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: Box::new(int_lit(2)),
            right: Box::new(int_lit(3)),
        },
        Span::dummy(),
    );

    let mul_expr = Expr::new(
        ExprKind::Binary {
            op: verum_ast::expr::BinOp::Mul,
            left: Box::new(add_expr),
            right: Box::new(int_lit(4)),
        },
        Span::dummy(),
    );

    let sub_expr = Expr::new(
        ExprKind::Binary {
            op: verum_ast::expr::BinOp::Sub,
            left: Box::new(mul_expr),
            right: Box::new(int_lit(1)),
        },
        Span::dummy(),
    );

    let repeat_expr = Expr::new(
        ExprKind::Array(ArrayExpr::Repeat {
            value: Box::new(sub_expr),
            count: Box::new(int_lit(10)),
        }),
        Span::dummy(),
    );

    let module = create_test_module(repeat_expr);

    // Should evaluate: (2 + 3) * 4 - 1 = 19
    let result = ctx.lower_module(&module);
    assert!(result.is_ok(), "Complex const expression should evaluate");
}

/// Test 8: Array size zero edge case
#[test]
fn test_array_size_zero() {
    let mut ctx = LoweringContext::new();

    // Create array type: [i32; 0]
    let array_type = Type {
        kind: TypeKind::Array {
            element: Box::new(Type {
                kind: TypeKind::Int,
                span: Span::dummy(),
            }),
            size: Some(Box::new(int_lit(0))),
        },
        span: Span::dummy(),
    };

    let mir_type = ctx.lower_type(&array_type);

    // Verify zero-sized array
    match mir_type {
        MirType::Array(_elem, size) => {
            assert_eq!(size, 0, "Array size should be 0");
        }
        _ => panic!("Expected MirType::Array, got {:?}", mir_type),
    }
}

/// Test 9: Large const array size
#[test]
fn test_large_array_size() {
    let mut ctx = LoweringContext::new();

    // Create array type: [i32; 1000]
    let array_type = Type {
        kind: TypeKind::Array {
            element: Box::new(Type {
                kind: TypeKind::Int,
                span: Span::dummy(),
            }),
            size: Some(Box::new(int_lit(1000))),
        },
        span: Span::dummy(),
    };

    let mir_type = ctx.lower_type(&array_type);

    // Verify large array size
    match mir_type {
        MirType::Array(_elem, size) => {
            assert_eq!(size, 1000, "Array size should be 1000");
        }
        _ => panic!("Expected MirType::Array, got {:?}", mir_type),
    }
}

/// Test 10: Performance - const evaluation should be fast
#[test]
fn test_const_eval_performance() {
    use std::time::Instant;

    let mut ctx = LoweringContext::new();

    // Create array type: [i32; 100]
    let array_type = Type {
        kind: TypeKind::Array {
            element: Box::new(Type {
                kind: TypeKind::Int,
                span: Span::dummy(),
            }),
            size: Some(Box::new(int_lit(100))),
        },
        span: Span::dummy(),
    };

    let start = Instant::now();
    let _mir_type = ctx.lower_type(&array_type);
    let duration = start.elapsed();

    // Const evaluation should be < 10ms (target from requirements)
    assert!(
        duration.as_millis() < 10,
        "Const evaluation took {:?}, should be < 10ms",
        duration
    );
}
