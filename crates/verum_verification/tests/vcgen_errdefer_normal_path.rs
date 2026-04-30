//! Regression: `vcgen.rs::wp_stmt` for `StmtKind::Errdefer` is a
//! no-op for normal-path WP — same architectural fix as
//! `verum_smt::wp_calculus` (commit fc02bfc9, task #24).
//!
//! Pre-fix the arm called `wp_expr(expr, postcondition)` — the
//! semantics of `defer` (always runs) — which wrongly threaded the
//! cleanup's effects through every normal exit. Post-fix the arm
//! returns `postcondition.clone()` and binds the expression to `_`.
//!
//! The pin: build two Stmts producing the same value, one
//! `errdefer cleanup_expr;` and one Empty. Their WPs against the
//! same postcondition must be equal.

#![allow(dead_code)]

use verum_ast::expr::Expr;
use verum_ast::span::Span;
use verum_ast::stmt::StmtKind;
use verum_ast::ty::{Ident, Path};
use verum_ast::Stmt;

use verum_verification::vcgen::{Formula, VCGenerator};

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn errdefer_stmt(cleanup: Expr) -> Stmt {
    Stmt {
        kind: StmtKind::Errdefer(cleanup),
        span: Span::dummy(),
        attributes: Vec::new(),
    }
}

fn empty_stmt() -> Stmt {
    Stmt {
        kind: StmtKind::Empty,
        span: Span::dummy(),
        attributes: Vec::new(),
    }
}

#[test]
fn vcgen_errdefer_does_not_affect_normal_path_wp() {
    let mut vcgen = VCGenerator::new();
    // Postcondition: the constant `true`. Any normal-path WP for
    // a no-op stmt should return the same Formula::True.
    let postcond = Formula::True;

    let wp_with_errdefer = vcgen.wp_stmt(&errdefer_stmt(ident_expr("buf")), &postcond);
    let wp_empty = vcgen.wp_stmt(&empty_stmt(), &postcond);

    assert_eq!(
        format!("{:?}", wp_with_errdefer),
        format!("{:?}", wp_empty),
        "errdefer must be a no-op for normal-path WP — pre-fix this propagated cleanup as if it were defer"
    );
}
