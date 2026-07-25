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
//! Comprehensive tests for statement AST nodes
//!
//! Tests cover all statement types:
//! - Let bindings
//! - Let-else statements
//! - Expression statements
//! - Item declarations within blocks
//! - Defer statements (RAII cleanup)
//! - Provide statements (context injection)
//! - Empty statements
//!
//! Comprehensive tests for statement AST nodes.

use verum_ast::*;
use verum_common::{Heap, Maybe, Text};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name, test_span())
}

// ============================================================================
// LET STATEMENT TESTS
// ============================================================================

#[test]
fn test_let_stmt_with_value() {
    let span = test_span();
    let pattern = Pattern::ident(test_ident("x"), false, span);
    let value = Maybe::Some(Expr::literal(Literal::int(42, span)));

    let stmt = Stmt::let_stmt(pattern.clone(), Maybe::None, value, span);

    match &stmt.kind {
        StmtKind::Let {
            pattern: p,
            value: v,
            ..
        } => {
            assert_eq!(p, &pattern);
            assert!(matches!(v, &Maybe::Some(_)));
        }
        _ => panic!("Expected Let statement"),
    }
    assert_eq!(stmt.span, span);
}

#[test]
fn test_let_stmt_with_type() {
    let span = test_span();
    let pattern = Pattern::ident(test_ident("x"), false, span);
    let ty = Maybe::Some(Type::int(span));
    let value = Maybe::Some(Expr::literal(Literal::int(42, span)));

    let stmt = Stmt::let_stmt(pattern, ty.clone(), value, span);

    match &stmt.kind {
        StmtKind::Let { ty: t, .. } => {
            assert!(matches!(t, &Maybe::Some(_)));
        }
        _ => panic!("Expected Let statement"),
    }
}

#[test]
fn test_let_stmt_without_value() {
    let span = test_span();
    let pattern = Pattern::ident(test_ident("x"), false, span);
    let ty = Maybe::Some(Type::int(span));

    let stmt = Stmt::let_stmt(pattern, ty, Maybe::None, span);

    match &stmt.kind {
        StmtKind::Let { value, .. } => {
            assert!(matches!(value, &Maybe::None));
        }
        _ => panic!("Expected Let statement"),
    }
}

#[test]
fn test_let_stmt_mutable_pattern() {
    let span = test_span();
    let pattern = Pattern::ident(test_ident("x"), true, span);
    let value = Maybe::Some(Expr::literal(Literal::int(42, span)));

    let stmt = Stmt::let_stmt(pattern.clone(), Maybe::None, value, span);

    match &stmt.kind {
        StmtKind::Let { pattern, .. } => match &pattern.kind {
            PatternKind::Ident { mutable, .. } => {
                assert!(*mutable);
            }
            _ => panic!("Expected Ident pattern"),
        },
        _ => panic!("Expected Let statement"),
    }
}

// ============================================================================
// LET-ELSE STATEMENT TESTS
// ============================================================================

#[test]
fn test_let_else_stmt() {
    let span = test_span();
    let pattern = Pattern::ident(test_ident("x"), false, span);
    let value = Expr::ident(test_ident("opt"));
    let else_block = Block::empty(span);

    let stmt = Stmt::new(
        StmtKind::LetElse {
            pattern: pattern.clone(),
            ty: Maybe::None,
            value: value.clone(),
            else_block: else_block.clone(),
        },
        span,
    );

    match &stmt.kind {
        StmtKind::LetElse {
            pattern: p,
            value: v,
            ..
        } => {
            assert_eq!(p, &pattern);
            assert_eq!(v, &value);
        }
        _ => panic!("Expected LetElse statement"),
    }
}

#[test]
fn test_let_else_stmt_with_type() {
    let span = test_span();
    let pattern = Pattern::ident(test_ident("x"), false, span);
    let ty = Maybe::Some(Type::int(span));
    let value = Expr::ident(test_ident("opt"));
    let else_block = Block::empty(span);

    let stmt = Stmt::new(
        StmtKind::LetElse {
            pattern,
            ty: ty.clone(),
            value,
            else_block,
        },
        span,
    );

    match &stmt.kind {
        StmtKind::LetElse { ty: t, .. } => {
            assert!(matches!(t, &Maybe::Some(_)));
        }
        _ => panic!("Expected LetElse statement"),
    }
}

// ============================================================================
// EXPRESSION STATEMENT TESTS
// ============================================================================

#[test]
fn test_expr_stmt_with_semi() {
    let span = test_span();
    let expr = Expr::literal(Literal::int(42, span));

    let stmt = Stmt::expr(expr.clone(), true);

    match &stmt.kind {
        StmtKind::Expr { expr: e, has_semi } => {
            assert_eq!(e, &expr);
            assert!(*has_semi);
        }
        _ => panic!("Expected Expr statement"),
    }
}

#[test]
fn test_expr_stmt_without_semi() {
    let span = test_span();
    let expr = Expr::literal(Literal::int(42, span));

    let stmt = Stmt::expr(expr.clone(), false);

    match &stmt.kind {
        StmtKind::Expr { has_semi, .. } => {
            assert!(!has_semi);
        }
        _ => panic!("Expected Expr statement"),
    }
}

// ============================================================================
// DEFER STATEMENT TESTS (RAII cleanup)
// ============================================================================

#[test]
fn test_defer_stmt() {
    let span = test_span();
    let expr = Expr::ident(test_ident("cleanup"));

    let stmt = Stmt::new(StmtKind::Defer(expr.clone()), span);

    match &stmt.kind {
        StmtKind::Defer(e) => {
            assert_eq!(e, &expr);
        }
        _ => panic!("Expected Defer statement"),
    }
}

// ============================================================================
// PROVIDE STATEMENT TESTS (Context injection)
// ============================================================================

#[test]
fn test_provide_stmt() {
    let span = test_span();
    let context = Text::from("Database");
    let value = Heap::new(Expr::ident(test_ident("db")));

    let stmt = Stmt::new(
        StmtKind::Provide {
            context: context.clone(),
            alias: Maybe::None,
            value: value.clone(),
        },
        span,
    );

    match &stmt.kind {
        StmtKind::Provide { context: ctx, .. } => {
            assert_eq!(ctx, &context);
        }
        _ => panic!("Expected Provide statement"),
    }
}

// ============================================================================
// EMPTY STATEMENT TEST
// ============================================================================

#[test]
fn test_empty_stmt() {
    let span = test_span();
    let stmt = Stmt::new(StmtKind::Empty, span);

    assert!(matches!(&stmt.kind, &StmtKind::Empty));
}

// ============================================================================
// SAFETY TESTS - No panics
// ============================================================================

#[test]
fn test_stmt_construction_never_panics() {
    let span = test_span();

    // All statement constructors should work
    let _ = Stmt::let_stmt(
        Pattern::ident(test_ident("x"), false, span),
        Maybe::None,
        Maybe::Some(Expr::literal(Literal::int(42, span))),
        span,
    );

    let _ = Stmt::expr(Expr::literal(Literal::int(42, span)), true);

    let _ = Stmt::new(StmtKind::Defer(Expr::ident(test_ident("cleanup"))), span);

    let _ = Stmt::new(
        StmtKind::Provide {
            context: Text::from("Context"),
            alias: Maybe::None,
            value: Heap::new(Expr::ident(test_ident("value"))),
        },
        span,
    );

    let _ = Stmt::new(StmtKind::Empty, span);
}
