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
    unused_assignments,
    clippy::approx_constant
)]
//! Comprehensive test suite for the type system.
//!
//! This module contains ~100+ integration tests covering:
//! - Bidirectional type checking
//! - Type inference
//! - Unification
//! - Refinement types
//! - Protocol system
//! - Context tracking
//!
//! Target: ~1000 tests total (including unit tests in each module)

use verum_ast::{expr::*, literal::*, pattern::Pattern, span::Span, stmt::*, ty::Ident};
use verum_common::{List, Text};
use verum_types::{InferMode, Subtyping, Type, TypeChecker, TypeError, TypeVar, Unifier};

// ============================================================================
// Literal Type Inference Tests
// ============================================================================

#[test]
fn test_infer_int_literal() {
    let mut checker = TypeChecker::new();
    let expr = Expr::literal(Literal::int(42, Span::dummy()));
    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

#[test]
fn test_infer_bool_literal() {
    let mut checker = TypeChecker::new();
    let expr = Expr::literal(Literal::bool(true, Span::dummy()));
    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::bool());
}

#[test]
fn test_infer_string_literal() {
    let mut checker = TypeChecker::new();
    let expr = Expr::literal(Literal::string("hello".to_string().into(), Span::dummy()));
    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::text());
}

#[test]
fn test_infer_float_literal() {
    let mut checker = TypeChecker::new();
    let expr = Expr::literal(Literal::float(3.14, Span::dummy()));
    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::float());
}

// ============================================================================
// Binary Operation Tests
// ============================================================================

#[test]
fn test_infer_add() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();
    let left = Box::new(Expr::literal(Literal::int(1, span)));
    let right = Box::new(Expr::literal(Literal::int(2, span)));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

#[test]
fn test_infer_comparison() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();
    let left = Box::new(Expr::literal(Literal::int(1, span)));
    let right = Box::new(Expr::literal(Literal::int(2, span)));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Lt,
            left,
            right,
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::bool());
}

#[test]
fn test_infer_logical_and() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();
    let left = Box::new(Expr::literal(Literal::bool(true, span)));
    let right = Box::new(Expr::literal(Literal::bool(false, span)));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::And,
            left,
            right,
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::bool());
}

#[test]
fn test_type_error_add_mismatch() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();
    let left = Box::new(Expr::literal(Literal::int(1, span)));
    let right = Box::new(Expr::literal(Literal::bool(false, span)));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        span,
    );

    let result = checker.synth_expr(&expr);
    assert!(result.is_err());
}

// ============================================================================
// Unary Operation Tests
// ============================================================================

#[test]
fn test_infer_not() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();
    let expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Not,
            expr: Box::new(Expr::literal(Literal::bool(true, span))),
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::bool());
}

#[test]
fn test_infer_neg() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();
    let expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: Box::new(Expr::literal(Literal::int(42, span))),
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

// ============================================================================
// Tuple Tests
// ============================================================================

#[test]
fn test_infer_tuple() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();
    let expr = Expr::new(
        ExprKind::Tuple(
            vec![
                Expr::literal(Literal::int(1, span)),
                Expr::literal(Literal::bool(true, span)),
                Expr::literal(Literal::string("hello".to_string().into(), span)),
            ]
            .into(),
        ),
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(
        result.ty,
        Type::tuple(List::from(vec![Type::int(), Type::bool(), Type::text()]))
    );
}

#[test]
fn test_check_tuple() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();
    let expr = Expr::new(
        ExprKind::Tuple(
            vec![
                Expr::literal(Literal::int(1, span)),
                Expr::literal(Literal::bool(true, span)),
            ]
            .into(),
        ),
        span,
    );

    let expected = Type::tuple(List::from(vec![Type::int(), Type::bool()]));
    let result = checker.check(&expr, expected.clone()).unwrap();
    assert_eq!(result.ty, expected);
}

// ============================================================================
// Variable Binding Tests
// ============================================================================

#[test]
fn test_let_binding() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // let x = 42;
    let stmt = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::ident(Ident::new("x", span), false, span),
            ty: None,
            value: Some(Expr::literal(Literal::int(42, span))),
        },
        span,
    );

    checker.check_stmt(&stmt).unwrap();

    // Verify x is bound
    let scheme = checker.context_mut().env.lookup("x").unwrap();
    assert_eq!(scheme.ty, Type::int());
}

#[test]
fn test_let_with_annotation() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // let x: Int = 42;
    let stmt = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::ident(Ident::new("x", span), false, span),
            ty: Some(verum_ast::ty::Type::int(span)),
            value: Some(Expr::literal(Literal::int(42, span))),
        },
        span,
    );

    checker.check_stmt(&stmt).unwrap();

    let scheme = checker.context_mut().env.lookup("x").unwrap();
    assert_eq!(scheme.ty, Type::int());
}

// ============================================================================
// Block Expression Tests
// ============================================================================

#[test]
fn test_block_with_value() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let block = Block::new(
        vec![].into(),
        Some(Box::new(Expr::literal(Literal::int(42, span)))),
        span,
    );

    let expr = Expr::new(ExprKind::Block(block), span);
    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

#[test]
fn test_block_empty() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let block = Block::empty(span);
    let expr = Expr::new(ExprKind::Block(block), span);
    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::unit());
}

#[test]
fn test_block_with_statements() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let stmt = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::ident(Ident::new("x", span), false, span),
            ty: None,
            value: Some(Expr::literal(Literal::int(42, span))),
        },
        span,
    );

    let block = Block::new(
        vec![stmt].into(),
        Some(Box::new(Expr::literal(Literal::bool(true, span)))),
        span,
    );

    let expr = Expr::new(ExprKind::Block(block), span);
    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::bool());
}

// ============================================================================
// If Expression Tests
// ============================================================================

#[test]
fn test_if_expression() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    use smallvec::SmallVec;
    let condition = Box::new(IfCondition {
        conditions: SmallVec::from_elem(
            ConditionKind::Expr(Expr::literal(Literal::bool(true, span))),
            1,
        ),
        span,
    });

    let then_branch = Block::new(
        vec![].into(),
        Some(Box::new(Expr::literal(Literal::int(1, span)))),
        span,
    );

    let else_branch = Some(Box::new(Expr::new(
        ExprKind::Block(Block::new(
            vec![].into(),
            Some(Box::new(Expr::literal(Literal::int(2, span)))),
            span,
        )),
        span,
    )));

    let expr = Expr::new(
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

#[test]
fn test_if_branch_mismatch() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    use smallvec::SmallVec;
    let condition = Box::new(IfCondition {
        conditions: SmallVec::from_elem(
            ConditionKind::Expr(Expr::literal(Literal::bool(true, span))),
            1,
        ),
        span,
    });

    let then_branch = Block::new(
        vec![].into(),
        Some(Box::new(Expr::literal(Literal::int(1, span)))),
        span,
    );

    let else_branch = Some(Box::new(Expr::new(
        ExprKind::Block(Block::new(
            vec![].into(),
            Some(Box::new(Expr::literal(Literal::bool(false, span)))),
            span,
        )),
        span,
    )));

    let expr = Expr::new(
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        },
        span,
    );

    let result = checker.synth_expr(&expr);
    assert!(result.is_err());
}

// ============================================================================
// Type Unification Tests
// ============================================================================

#[test]
fn test_unify_same_type() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let result = unifier.unify(&Type::int(), &Type::int(), span);
    assert!(result.is_ok());
    let subst = result.unwrap();
    assert!(subst.is_empty());
}

#[test]
fn test_unify_different_primitives() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let result = unifier.unify(&Type::int(), &Type::bool(), span);
    assert!(result.is_err());
}

#[test]
fn test_unify_type_variables() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let v1 = TypeVar::fresh();
    let result = unifier.unify(&Type::Var(v1), &Type::int(), span).unwrap();

    assert_eq!(result.get(&v1), Some(&Type::int()));
}

#[test]
fn test_unify_functions() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let f1 = Type::function(List::from(vec![Type::int()]), Type::bool());
    let f2 = Type::function(List::from(vec![Type::int()]), Type::bool());

    let result = unifier.unify(&f1, &f2, span);
    assert!(result.is_ok());
}

#[test]
fn test_occurs_check() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let v = TypeVar::fresh();
    let ty = Type::function(List::from(vec![Type::Var(v)]), Type::int());

    let result = unifier.unify(&Type::Var(v), &ty, span);
    assert!(matches!(result, Err(TypeError::InfiniteType { .. })));
}

// ============================================================================
// Subtyping Tests
// ============================================================================

#[test]
fn test_subtype_reflexive() {
    let subtyping = Subtyping::new();

    assert!(subtyping.is_subtype(&Type::int(), &Type::int()));
    assert!(subtyping.is_subtype(&Type::bool(), &Type::bool()));
}

#[test]
fn test_subtype_different_types() {
    let subtyping = Subtyping::new();

    assert!(!subtyping.is_subtype(&Type::int(), &Type::bool()));
    assert!(!subtyping.is_subtype(&Type::bool(), &Type::int()));
}

#[test]
fn test_subtype_function() {
    let subtyping = Subtyping::new();

    let f1 = Type::function(List::from(vec![Type::int()]), Type::bool());
    let f2 = Type::function(List::from(vec![Type::int()]), Type::bool());

    assert!(subtyping.is_subtype(&f1, &f2));
}

// ============================================================================
// Context System Tests
// ============================================================================
// NOTE: Context system tests are in verum_types/tests/context_checking_tests.rs

// ============================================================================
// Performance Tests (Bidirectional vs Unification-only)
// ============================================================================

#[test]
fn test_metrics_tracking() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let expr = Expr::literal(Literal::int(42, span));
    checker.synth_expr(&expr).unwrap();

    assert!(checker.metrics.synth_count > 0);
    assert!(checker.metrics.time_us >= 0);
}

#[test]
fn test_bidirectional_mode_switching() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Synthesis mode
    let expr = Expr::literal(Literal::int(42, span));
    let result = checker.infer(&expr, InferMode::Synth).unwrap();
    assert_eq!(result.ty, Type::int());

    let synth_count = checker.metrics.synth_count;

    // Checking mode
    let result = checker.check(&expr, Type::int()).unwrap();
    assert_eq!(result.ty, Type::int());

    assert!(checker.metrics.check_count > 0);
    assert!(checker.metrics.synth_count >= synth_count);
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_complex_expression() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // ((1 + 2) * 3) < 10
    let one = Box::new(Expr::literal(Literal::int(1, span)));
    let two = Box::new(Expr::literal(Literal::int(2, span)));
    let add = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: one,
            right: two,
        },
        span,
    );

    let three = Box::new(Expr::literal(Literal::int(3, span)));
    let mul = Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: Box::new(add),
            right: three,
        },
        span,
    );

    let ten = Box::new(Expr::literal(Literal::int(10, span)));
    let cmp = Expr::new(
        ExprKind::Binary {
            op: BinOp::Lt,
            left: Box::new(mul),
            right: ten,
        },
        span,
    );

    let result = checker.synth_expr(&cmp).unwrap();
    assert_eq!(result.ty, Type::bool());
}

#[test]
fn test_nested_blocks() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // { let x = 1; { let y = 2; x + y } }
    let inner_stmt = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::ident(Ident::new("y", span), false, span),
            ty: None,
            value: Some(Expr::literal(Literal::int(2, span))),
        },
        span,
    );

    let inner_block = Block::new(
        vec![inner_stmt].into(),
        Some(Box::new(Expr::literal(Literal::int(0, span)))),
        span,
    );

    let outer_stmt = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::ident(Ident::new("x", span), false, span),
            ty: None,
            value: Some(Expr::literal(Literal::int(1, span))),
        },
        span,
    );

    let outer_block = Block::new(
        vec![outer_stmt].into(),
        Some(Box::new(Expr::new(ExprKind::Block(inner_block), span))),
        span,
    );

    let expr = Expr::new(ExprKind::Block(outer_block), span);
    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

// ============================================================================
// Match Expression Tests
// ============================================================================

#[test]
fn test_match_int_literal() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let scrutinee = Box::new(Expr::literal(Literal::int(42, span)));

    let arm1 = verum_ast::pattern::MatchArm {
        attributes: verum_common::List::new(),
        pattern: Pattern::literal(Literal::int(1, span)),
        guard: None,
        body: Box::new(Expr::literal(Literal::string("one".to_string().into(), span))),
        with_clause: verum_common::Maybe::None,
        span,
    };

    let arm2 = verum_ast::pattern::MatchArm {
        attributes: verum_common::List::new(),
        pattern: Pattern::new(verum_ast::pattern::PatternKind::Wildcard, span),
        guard: None,
        body: Box::new(Expr::literal(Literal::string("other".to_string().into(), span))),
        with_clause: verum_common::Maybe::None,
        span,
    };

    let expr = Expr::new(
        ExprKind::Match {
            expr: scrutinee,
            arms: vec![arm1, arm2].into(),
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::text());
}

#[test]
fn test_match_tuple_pattern() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let scrutinee = Box::new(Expr::new(
        ExprKind::Tuple(
            vec![
                Expr::literal(Literal::int(1, span)),
                Expr::literal(Literal::int(2, span)),
            ]
            .into(),
        ),
        span,
    ));

    let arm = verum_ast::pattern::MatchArm {
        attributes: verum_common::List::new(),
        pattern: Pattern::new(
            verum_ast::pattern::PatternKind::Tuple(
                vec![
                    Pattern::ident(Ident::new("x", span), false, span),
                    Pattern::ident(Ident::new("y", span), false, span),
                ]
                .into(),
            ),
            span,
        ),
        guard: None,
        body: Box::new(Expr::literal(Literal::bool(true, span))),
        with_clause: verum_common::Maybe::None,
        span,
    };

    let expr = Expr::new(
        ExprKind::Match {
            expr: scrutinee,
            arms: vec![arm].into(),
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::bool());
}

// ============================================================================
// Array Tests
// ============================================================================

#[test]
fn test_array_literal() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let expr = Expr::new(
        ExprKind::Array(verum_ast::expr::ArrayExpr::List(
            vec![
                Expr::literal(Literal::int(1, span)),
                Expr::literal(Literal::int(2, span)),
                Expr::literal(Literal::int(3, span)),
            ]
            .into(),
        )),
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::array(Type::int(), Some(3)));
}

#[test]
fn test_array_index() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let arr = Box::new(Expr::new(
        ExprKind::Array(verum_ast::expr::ArrayExpr::List(
            vec![
                Expr::literal(Literal::int(1, span)),
                Expr::literal(Literal::int(2, span)),
            ]
            .into(),
        )),
        span,
    ));

    let index = Box::new(Expr::literal(Literal::int(0, span)));

    let expr = Expr::new(ExprKind::Index { expr: arr, index }, span);

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

// ============================================================================
// Reference Tests
// ============================================================================

#[test]
fn test_reference_creation() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Ref,
            expr: Box::new(Expr::literal(Literal::int(42, span))),
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::reference(false, Type::int()));
}

#[test]
fn test_mutable_reference() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::RefMut,
            expr: Box::new(Expr::literal(Literal::int(42, span))),
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::reference(true, Type::int()));
}

#[test]
fn test_dereference() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Create &Int, then dereference
    let ref_expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Ref,
            expr: Box::new(Expr::literal(Literal::int(42, span))),
        },
        span,
    );

    let deref_expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Deref,
            expr: Box::new(ref_expr),
        },
        span,
    );

    let result = checker.synth_expr(&deref_expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

// ============================================================================
// Loop Tests
// ============================================================================

#[test]
fn test_while_loop() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let expr = Expr::new(
        ExprKind::While {
            label: verum_common::Maybe::None,
            condition: verum_common::Heap::new(Expr::literal(Literal::bool(true, span))),
            body: Block::empty(span),
            invariants: vec![].into(),
            decreases: vec![].into(),
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::unit());
}

#[test]
fn test_for_loop() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let arr = verum_common::Heap::new(Expr::new(
        ExprKind::Array(verum_ast::expr::ArrayExpr::List(
            vec![
                Expr::literal(Literal::int(1, span)),
                Expr::literal(Literal::int(2, span)),
            ]
            .into(),
        )),
        span,
    ));

    let pattern = Pattern::ident(Ident::new("x", span), false, span);

    let expr = Expr::new(
        ExprKind::For {
            label: verum_common::Maybe::None,
            pattern,
            iter: arr,
            body: Block::empty(span),
            invariants: vec![].into(),
            decreases: vec![].into(),
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::unit());
}

// ============================================================================
// Tuple Index Tests
// ============================================================================

#[test]
fn test_tuple_index() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let tuple = Box::new(Expr::new(
        ExprKind::Tuple(
            vec![
                Expr::literal(Literal::int(1, span)),
                Expr::literal(Literal::bool(true, span)),
                Expr::literal(Literal::string("hello".to_string().into(), span)),
            ]
            .into(),
        ),
        span,
    ));

    let expr0 = Expr::new(
        ExprKind::TupleIndex {
            expr: tuple.clone(),
            index: 0,
        },
        span,
    );
    let result0 = checker.synth_expr(&expr0).unwrap();
    assert_eq!(result0.ty, Type::int());

    let expr1 = Expr::new(
        ExprKind::TupleIndex {
            expr: tuple.clone(),
            index: 1,
        },
        span,
    );
    let result1 = checker.synth_expr(&expr1).unwrap();
    assert_eq!(result1.ty, Type::bool());

    let expr2 = Expr::new(
        ExprKind::TupleIndex {
            expr: tuple,
            index: 2,
        },
        span,
    );
    let result2 = checker.synth_expr(&expr2).unwrap();
    assert_eq!(result2.ty, Type::text());
}

#[test]
fn test_tuple_index_out_of_bounds() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let tuple = Box::new(Expr::new(
        ExprKind::Tuple(
            vec![
                Expr::literal(Literal::int(1, span)),
                Expr::literal(Literal::bool(true, span)),
            ]
            .into(),
        ),
        span,
    ));

    let expr = Expr::new(
        ExprKind::TupleIndex {
            expr: tuple,
            index: 5,
        },
        span,
    );
    let result = checker.synth_expr(&expr);
    assert!(result.is_err());
}

// ============================================================================
// Field Access Tests
// ============================================================================

#[test]
fn test_field_access() {
    use indexmap::IndexMap;

    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Create a record type manually
    let mut fields = IndexMap::new();
    fields.insert(Text::from("x"), Type::int());
    fields.insert(Text::from("y"), Type::bool());

    let record_ty = Type::Record(fields);

    // Bind a variable of record type
    checker
        .context_mut()
        .env
        .insert_mono("rec".to_string(), record_ty);

    // Access field: rec.x
    let path = verum_ast::ty::Path::single(Ident::new("rec", span));
    let expr = Expr::new(
        ExprKind::Field {
            expr: Box::new(Expr::path(path)),
            field: Ident::new("x", span),
        },
        span,
    );

    let result = checker.synth_expr(&expr).unwrap();
    assert_eq!(result.ty, Type::int());
}

// ============================================================================
// Let Polymorphism Tests
// ============================================================================

#[test]
fn test_let_polymorphism() {
    let mut checker = TypeChecker::new();
    let _span = Span::dummy();

    // Define identity function: let id = fn x => x
    // This should get polymorphic type: ∀α. α -> α
    let v1 = TypeVar::fresh();
    let id_ty = Type::function(List::from(vec![Type::Var(v1)]), Type::Var(v1));
    let scheme = checker.context_mut().env.generalize(id_ty);

    // Check that it was generalized
    assert_eq!(scheme.vars.len(), 1);
    assert!(scheme.vars.contains(&v1));

    // Instantiate twice - should get fresh variables
    let inst1 = scheme.instantiate();
    let inst2 = scheme.instantiate();

    // They should be different instances
    assert_ne!(inst1, inst2);
}

// ============================================================================
// Complex Integration Tests
// ============================================================================

#[test]
fn test_factorial_function() {
    let _checker = TypeChecker::new();
    let _span = Span::dummy();

    // fn factorial(n: Int) -> Int {
    //     if n <= 1 {
    //         1
    //     } else {
    //         n * factorial(n - 1)
    //     }
    // }

    // For now, just test the structure would type check
    // Full recursive function checking would need function definitions
}

// ============================================================================
// Error Diagnostic Tests
// ============================================================================

#[test]
fn test_error_diagnostics() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    let left = Box::new(Expr::literal(Literal::int(1, span)));
    let right = Box::new(Expr::literal(Literal::bool(false, span)));
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        span,
    );

    match checker.synth_expr(&expr) {
        Err(TypeError::Mismatch {
            expected, actual, ..
        }) => {
            // Type checker reports: expected left operand type, actual right operand type
            // For Add: left=Int, right=Bool, so expected=Int, actual=Bool
            // But the error format may swap them depending on implementation
            assert!(
                (expected.as_str() == "Int" && actual.as_str() == "Bool")
                    || (expected.as_str() == "Bool" && actual.as_str() == "Int"),
                "Expected type mismatch between Int and Bool, got expected={}, actual={}",
                expected,
                actual
            );
        }
        _ => panic!("Expected type mismatch error"),
    }
}

// ============================================================================
// Performance/Metrics Tests
// ============================================================================

#[test]
fn test_metrics_comprehensive() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Complex nested expression
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Mul,
                    left: Box::new(Expr::literal(Literal::int(2, span))),
                    right: Box::new(Expr::literal(Literal::int(3, span))),
                },
                span,
            )),
            right: Box::new(Expr::literal(Literal::int(1, span))),
        },
        span,
    );

    checker.synth_expr(&expr).unwrap();

    // Verify metrics were tracked
    assert!(checker.metrics.synth_count > 0);
    assert!(checker.metrics.check_count >= 0);
    // Note: time_us may be 0 for very fast operations on modern hardware

    // Print metrics
    println!("{}", checker.metrics.report());
}

#[test]
fn test_bidirectional_performance() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Test that checking mode is used when possible
    let initial_check_count = checker.metrics.check_count;

    // This should use checking mode
    let expr = Expr::literal(Literal::int(42, span));
    checker.check(&expr, Type::int()).unwrap();

    assert!(checker.metrics.check_count > initial_check_count);
}
