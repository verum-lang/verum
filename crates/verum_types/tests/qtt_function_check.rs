//! Integration test: `TypeChecker::check_function_qtt` runs the
//! walker + validator end-to-end on synthesized AST function
//! bodies, confirming the QTT analysis pipeline detects Linear,
//! Affine, and Omega misuses.

use std::collections::HashMap;

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Text};

use verum_types::infer::TypeChecker;
use verum_types::qtt_usage::ViolationKind;
use verum_types::ty::Quantity;

fn sp() -> Span {
    Span::default()
}

fn ident(name: &str) -> Ident {
    Ident::new(name, sp())
}

fn path_expr(name: &str) -> Expr {
    Expr {
        kind: ExprKind::Path(Path::single(ident(name))),
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
    }
}

fn add_xy() -> Expr {
    Expr {
        kind: ExprKind::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: verum_common::Heap::new(path_expr("x")),
            right: verum_common::Heap::new(path_expr("y")),
        },
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
    }
}

#[test]
fn linear_used_once_passes() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("x"), Quantity::One);
    let body = path_expr("x");
    let result = tc.check_function_qtt(&decl, &body);
    assert!(result.is_ok());
}

#[test]
fn linear_unused_fails() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("leaked"), Quantity::One);
    // body doesn't mention `leaked`
    let body = path_expr("other");
    let err = tc.check_function_qtt(&decl, &body).unwrap_err();
    assert_eq!(err.kind, ViolationKind::UnderUse);
    assert_eq!(err.binding.as_str(), "leaked");
}

#[test]
fn linear_used_twice_fails() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("x"), Quantity::One);
    // x + x — used twice
    let body = Expr {
        kind: ExprKind::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: verum_common::Heap::new(path_expr("x")),
            right: verum_common::Heap::new(path_expr("x")),
        },
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
    };
    let err = tc.check_function_qtt(&decl, &body).unwrap_err();
    assert_eq!(err.kind, ViolationKind::OverUse);
}

#[test]
fn omega_used_anywhere_passes() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("x"), Quantity::Omega);
    decl.insert(Text::from("y"), Quantity::Omega);
    // x + y, x + y, x + y — all uses of Omega bindings are fine
    let body = Expr {
        kind: ExprKind::Tuple(List::from_iter([
            add_xy(),
            add_xy(),
            add_xy(),
        ])),
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
    };
    assert!(tc.check_function_qtt(&decl, &body).is_ok());
}

#[test]
fn affine_two_uses_passes() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("x"), Quantity::AtMost(2));
    let body = Expr {
        kind: ExprKind::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: verum_common::Heap::new(path_expr("x")),
            right: verum_common::Heap::new(path_expr("x")),
        },
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
    };
    assert!(tc.check_function_qtt(&decl, &body).is_ok());
}

#[test]
fn zero_quantity_at_runtime_fails() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("ghost"), Quantity::Zero);
    let body = path_expr("ghost");
    let err = tc.check_function_qtt(&decl, &body).unwrap_err();
    assert_eq!(err.kind, ViolationKind::ErasedUsedAtRuntime);
}

#[test]
fn returned_usage_map_records_observed_counts() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("x"), Quantity::Omega);
    decl.insert(Text::from("y"), Quantity::Omega);
    let body = add_xy();
    let usage = tc.check_function_qtt(&decl, &body).unwrap();
    assert_eq!(usage.lookup(&Text::from("x")).runtime, 1);
    assert_eq!(usage.lookup(&Text::from("y")).runtime, 1);
}
