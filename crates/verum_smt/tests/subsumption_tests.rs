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
// Tests for subsumption module
// Migrated from src/subsumption.rs per CLAUDE.md standards

use verum_smt::subsumption::{CheckMode, SubsumptionChecker, SubsumptionResult};

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::{Literal, LiteralKind};
use verum_ast::span::Span;
use verum_common::Heap;

fn make_bool(b: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Bool(b),
            span: Span::dummy(),
        }),
        Span::dummy(),
    )
}

fn make_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::dummy(),
    )
}

#[test]
fn test_reflexivity() {
    let checker = SubsumptionChecker::new();
    let expr = make_bool(true);

    let result = checker.check(&expr, &expr, CheckMode::SyntacticOnly);
    assert!(result.is_valid());
    assert!(matches!(result, SubsumptionResult::Syntactic(true)));
}

#[test]
fn test_tautology() {
    let checker = SubsumptionChecker::new();
    let true_expr = make_bool(true);
    let false_expr = make_bool(false);

    // true => true
    let result = checker.check(&true_expr, &true_expr, CheckMode::SyntacticOnly);
    assert!(result.is_valid());

    // false => anything
    let result = checker.check(&false_expr, &true_expr, CheckMode::SyntacticOnly);
    assert!(result.is_valid());

    // anything => true
    let result = checker.check(&false_expr, &true_expr, CheckMode::SyntacticOnly);
    assert!(result.is_valid());
}

#[test]
fn test_conjunction() {
    let checker = SubsumptionChecker::new();
    let a = make_bool(true);
    let b = make_bool(false);
    let a_and_b = make_binary(BinOp::And, a.clone(), b.clone());

    // (a && b) => a
    let result = checker.check(&a_and_b, &a, CheckMode::SyntacticOnly);
    assert!(result.is_valid());

    // (a && b) => b
    let result = checker.check(&a_and_b, &b, CheckMode::SyntacticOnly);
    assert!(result.is_valid());
}

#[test]
fn test_disjunction() {
    let checker = SubsumptionChecker::new();
    let a = make_bool(true);
    let b = make_bool(false);
    let a_or_b = make_binary(BinOp::Or, a.clone(), b.clone());

    // a => (a || b)
    let result = checker.check(&a, &a_or_b, CheckMode::SyntacticOnly);
    assert!(result.is_valid());

    // b => (a || b)
    let result = checker.check(&b, &a_or_b, CheckMode::SyntacticOnly);
    assert!(result.is_valid());
}

#[test]
fn test_cache() {
    let checker = SubsumptionChecker::new();

    // Create complex expressions that will require SMT checking (not syntactic)
    // Use different variables so syntactic checking won't handle it
    let x = Expr::new(
        ExprKind::Path(verum_ast::ty::Path::single(verum_ast::ty::Ident {
            name: "x".into(),
            span: Span::dummy(),
        })),
        Span::dummy(),
    );
    let y = Expr::new(
        ExprKind::Path(verum_ast::ty::Path::single(verum_ast::ty::Ident {
            name: "y".into(),
            span: Span::dummy(),
        })),
        Span::dummy(),
    );

    // First check - cache miss, goes to SMT
    let _result1 = checker.check(&x, &y, CheckMode::SmtAllowed);

    // Second check - should hit cache
    let _result2 = checker.check(&x, &y, CheckMode::SmtAllowed);

    let stats = checker.stats();
    // Should have 1 cache hit from the second check
    assert!(
        stats.cache_hits >= 1,
        "Expected at least 1 cache hit, got {}",
        stats.cache_hits
    );
    assert!(stats.cache_hit_rate() > 0.0);
}
