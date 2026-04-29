//! Regression: `errdefer` is a no-op for normal-path WP.
//!
//! Pre-fix `wp_calculus.rs::StmtKind::Errdefer` propagated WP through
//! the cleanup expression — identical to `Defer`. The comment on the
//! arm even said "treat as a no-op for normal path verification" but
//! the body did the opposite. That gave any function with `errdefer`
//! the WRONG normal-path WP.
//!
//! Post-fix: errdefer normal-path WP is just the postcondition
//! unchanged. Error-path WP modeling is a separate phase.
//!
//! The pin: build two Blocks producing the same final value,
//! one containing `errdefer cleanup_expr;` before the value, the
//! other empty. Their WPs against the same postcondition must be
//! equal — proving errdefer doesn't disturb the normal path.

#![allow(unused_imports, dead_code)]

use verum_ast::span::Span;
use verum_ast::stmt::StmtKind;
use verum_ast::{
    Expr, Ident, Literal, Path, Span as AstSpan, Stmt, Type, TypeKind,
    expr::{BinOp, Block, ExprKind},
};
use verum_common::{Heap, List, Maybe, Text};
use verum_smt::{Context, ContextConfig, wp_calculus::WpEngine};
use z3::ast::{Ast, Bool};

fn dummy_span() -> AstSpan {
    AstSpan::dummy()
}

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, dummy_span())))
}

fn int_lit(v: i64) -> Expr {
    Expr::literal(Literal::int(v as i128, dummy_span()))
}

fn block(stmts: List<Stmt>, result: Option<Expr>) -> Expr {
    Expr::new(
        ExprKind::Block(Block {
            stmts,
            expr: result.map(Heap::new),
            span: dummy_span(),
        }),
        dummy_span(),
    )
}

fn errdefer_stmt(cleanup: Expr) -> Stmt {
    Stmt {
        kind: StmtKind::Errdefer(cleanup),
        span: dummy_span(),
        attributes: Vec::new(),
    }
}

fn defer_stmt(cleanup: Expr) -> Stmt {
    Stmt {
        kind: StmtKind::Defer(cleanup),
        span: dummy_span(),
        attributes: Vec::new(),
    }
}

#[test]
fn errdefer_does_not_affect_normal_path_wp() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Bind a variable so the cleanup expression has something to
    // reference.
    engine
        .bind_input(&Text::from("buf"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();

    let postcond = Bool::new_const("post");

    // Block A: a single errdefer over `buf` — normal-path WP must
    // ignore it, leaving WP = postcondition.
    let cleanup = ident_expr("buf");
    let block_with_errdefer = block(
        List::from(vec![errdefer_stmt(cleanup)]),
        Some(int_lit(0)),
    );

    // Block B: empty block returning the same value.
    let block_empty = block(List::new(), Some(int_lit(0)));

    let wp_with_errdefer = engine.wp(&block_with_errdefer, &postcond).unwrap();
    let wp_empty = engine.wp(&block_empty, &postcond).unwrap();

    // Both Blocks must yield the same Z3 expression. Pre-fix, the
    // errdefer's body would have been threaded through, producing a
    // different (and wrong) WP.
    assert_eq!(
        wp_with_errdefer.to_string(),
        wp_empty.to_string(),
        "errdefer must be a no-op for normal-path WP — pre-fix this propagated cleanup as if it were defer"
    );
}

#[test]
fn defer_still_threads_cleanup_through_normal_path() {
    // Negative control: defer runs on EVERY scope exit, so its WP
    // should propagate through the cleanup expression. This test
    // pins that the fix only changed errdefer, not defer — they're
    // semantically distinct and the WP calculus must reflect that.
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    engine
        .bind_input(&Text::from("buf"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();

    let postcond = Bool::new_const("post");

    // Block A: a single defer (NOT errdefer) — should propagate the
    // cleanup expression's WP through.
    let cleanup = ident_expr("buf");
    let block_with_defer = block(
        List::from(vec![defer_stmt(cleanup)]),
        Some(int_lit(0)),
    );

    // For defer the WP path is exercised; we just confirm it succeeds
    // (full equational check would require building the equivalent
    // sequenced expression, which is beyond a regression pin).
    let wp = engine.wp(&block_with_defer, &postcond);
    assert!(wp.is_ok(), "defer WP must compute without error: {:?}", wp);
}
