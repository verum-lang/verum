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
//! Tests for postcondition validation
//!
//! This test file verifies that the contract system properly validates
//! postconditions at compile-time.

use smallvec::SmallVec;
use verum_ast::{
    decl::{FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind},
    expr::{BinOp, Block, ConditionKind, Expr, ExprKind, IfCondition},
    literal::Literal,
    pattern::{Pattern, PatternKind},
    span::Span,
    ty::{Ident, Path, Type, TypeKind},
};
use verum_common::{Heap, List, Maybe};
use verum_types::{TypeContext, contracts::PostconditionValidator};

/// Helper to create a simple function with postcondition
fn make_abs_function() -> FunctionDecl {
    let span = Span::dummy();

    // Parameter: x: Int
    let param = FunctionParam::new(
        FunctionParamKind::Regular {
            pattern: Pattern {
                kind: PatternKind::Ident {
                    by_ref: false,
                    mutable: false,
                    name: Ident::new("x", span),
                    subpattern: None,
                },
                span,
            },
            ty: Type::new(
                TypeKind::Path(Path::from_ident(Ident::new("Int", span))),
                span,
            ),
            default_value: verum_common::Maybe::None,
        },
        span,
    );

    // Return type: Int
    let return_type = Type::new(
        TypeKind::Path(Path::from_ident(Ident::new("Int", span))),
        span,
    );

    // Postcondition: result >= 0
    let result_ident = Expr::ident(Ident::new("result", span));
    let zero = Expr::literal(Literal::int(0, span));
    let postcondition = Expr::new(
        ExprKind::Binary {
            op: BinOp::Ge,
            left: Heap::new(result_ident),
            right: Heap::new(zero),
        },
        span,
    );

    // Body: if x >= 0 { x } else { -x }
    let x_ident = Expr::ident(Ident::new("x", span));
    let zero_cond = Expr::literal(Literal::int(0, span));
    let condition = Expr::new(
        ExprKind::Binary {
            op: BinOp::Ge,
            left: Heap::new(x_ident.clone()),
            right: Heap::new(zero_cond),
        },
        span,
    );

    let then_branch = Block {
        stmts: List::new(),
        expr: Some(Heap::new(x_ident.clone())),
        span,
    };
    let else_branch = Expr::new(
        ExprKind::Unary {
            op: verum_ast::expr::UnOp::Neg,
            expr: Heap::new(x_ident),
        },
        span,
    );

    let if_condition = IfCondition {
        conditions: SmallVec::from_vec(vec![ConditionKind::Expr(condition)]),
        span,
    };

    let body_expr = Expr::new(
        ExprKind::If {
            condition: Heap::new(if_condition),
            then_branch,
            else_branch: Some(Heap::new(else_branch)),
        },
        span,
    );

    FunctionDecl {
        visibility: verum_ast::decl::Visibility::Private,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: None,
        name: Ident::new("abs", span),
        generics: vec![].into(),
        params: vec![param].into(),
        return_type: Some(return_type),
        throws_clause: None,
        std_attr: None,
        contexts: vec![].into(),
        generic_where_clause: None,
        meta_where_clause: None,
        requires: vec![].into(),
        ensures: vec![postcondition].into(),
        attributes: vec![].into(),
        body: Some(FunctionBody::Expr(body_expr)),
        span,
    }
}

#[test]
fn test_postcondition_validator_creation() {
    let _validator = PostconditionValidator::new();
    // If we get here, the validator was created successfully
}

#[test]
fn test_abs_function_postcondition_validation() {
    let func = make_abs_function();
    let return_type = verum_types::Type::int();
    let ctx = TypeContext::new();

    let mut validator = PostconditionValidator::new();

    // This should validate successfully (abs always returns non-negative)
    let result = validator.validate_postconditions(&func, &return_type, &ctx);

    // The current implementation is a stub that tracks stats but doesn't fail
    // This test ensures the API works correctly
    assert!(
        result.is_ok(),
        "Postcondition validation should succeed for abs function"
    );
    assert_eq!(
        validator.stats().postconditions_checked,
        1,
        "Should have checked one postcondition"
    );
}

#[test]
fn test_validator_stats_tracking() {
    // Test that the validator properly tracks statistics
    let func = make_abs_function();
    let return_type = verum_types::Type::int();
    let ctx = TypeContext::new();

    let mut validator = PostconditionValidator::new();

    // Initially no postconditions checked
    assert_eq!(validator.stats().postconditions_checked, 0);

    // Validate postconditions
    let _ = validator.validate_postconditions(&func, &return_type, &ctx);

    // Stats should be updated
    assert_eq!(
        validator.stats().postconditions_checked,
        1,
        "Should have tracked the postcondition check"
    );

    // Reset stats
    validator.reset_stats();
    assert_eq!(
        validator.stats().postconditions_checked,
        0,
        "Stats should be reset"
    );
}
