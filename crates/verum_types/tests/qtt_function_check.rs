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

// =============================================================================
// Red-team Round 1 §6.2 — Erased-vs-reified consistency exhaustive cases
// =============================================================================
//
// Round 1 §6.2 PARTIAL DEFENSE → DEFENSE CONFIRMED. The Quantity type
// system distinguishes Zero (erased, compile-time only) from Omega
// (reified, runtime-usable). Mixing them inconsistently — flowing an
// erased binding to a runtime position — surfaces as
// ViolationKind::ErasedUsedAtRuntime. The tests below cover the four
// canonical erasure-consistency scenarios.

/// Erased-zero binding alongside reified-omega binding — both flow
/// through their declared quantities; no cross-contamination.
#[test]
fn red_team_1_6_2_meta_zero_alongside_omega_consistent() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("compile_time_T"), Quantity::Zero);
    decl.insert(Text::from("runtime_x"), Quantity::Omega);
    // Body uses ONLY runtime_x; compile_time_T stays at Zero usage.
    let body = path_expr("runtime_x");
    let usage = tc.check_function_qtt(&decl, &body).unwrap();
    assert_eq!(usage.lookup(&Text::from("compile_time_T")).runtime, 0);
    assert_eq!(usage.lookup(&Text::from("runtime_x")).runtime, 1);
}

/// Erased binding ESCAPING to runtime — must surface as
/// ErasedUsedAtRuntime even when other bindings are well-formed.
/// This is the canonical adversarial case: an erased generic that
/// the compiler must NOT silently allow at runtime.
#[test]
fn red_team_1_6_2_meta_zero_escaping_to_runtime_caught() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("phantom_T"), Quantity::Zero);
    decl.insert(Text::from("real_x"), Quantity::Omega);
    // Body uses BOTH — phantom_T at runtime is the violation.
    let body = Expr {
        kind: ExprKind::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: verum_common::Heap::new(path_expr("phantom_T")),
            right: verum_common::Heap::new(path_expr("real_x")),
        },
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
    };
    let err = tc.check_function_qtt(&decl, &body).unwrap_err();
    assert_eq!(err.kind, ViolationKind::ErasedUsedAtRuntime);
    assert_eq!(err.binding.as_str(), "phantom_T");
}

/// Erased binding used twice at runtime — must STILL surface as
/// ErasedUsedAtRuntime (not as OverUse). The Zero-quantity check
/// is logically prior to the linearity check.
#[test]
fn red_team_1_6_2_meta_zero_used_multiple_times_still_erased_violation() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("ghost"), Quantity::Zero);
    // ghost + ghost — twice at runtime.
    let body = Expr {
        kind: ExprKind::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: verum_common::Heap::new(path_expr("ghost")),
            right: verum_common::Heap::new(path_expr("ghost")),
        },
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
    };
    let err = tc.check_function_qtt(&decl, &body).unwrap_err();
    assert_eq!(err.kind, ViolationKind::ErasedUsedAtRuntime);
}

/// Mixed quantity composition — Linear (One) used once + Omega
/// used three times + Zero unused. All three quantities co-exist
/// without cross-contamination.
#[test]
fn red_team_1_6_2_three_quantities_compose_consistently() {
    let tc = TypeChecker::new();
    let mut decl = HashMap::new();
    decl.insert(Text::from("erased_phantom"), Quantity::Zero);
    decl.insert(Text::from("linear_resource"), Quantity::One);
    decl.insert(Text::from("omega_value"), Quantity::Omega);
    // omega_value used 3 times; linear_resource used 1 time;
    // erased_phantom unused. Body: linear_resource + omega_value
    //                      + omega_value + omega_value (4 references)
    let omv = path_expr("omega_value");
    let outer_left = Expr {
        kind: ExprKind::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: verum_common::Heap::new(path_expr("linear_resource")),
            right: verum_common::Heap::new(omv.clone()),
        },
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
    };
    let outer_right = Expr {
        kind: ExprKind::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: verum_common::Heap::new(omv.clone()),
            right: verum_common::Heap::new(omv),
        },
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
    };
    let body = Expr {
        kind: ExprKind::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: verum_common::Heap::new(outer_left),
            right: verum_common::Heap::new(outer_right),
        },
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
    };
    let usage = tc.check_function_qtt(&decl, &body).unwrap();
    assert_eq!(usage.lookup(&Text::from("erased_phantom")).runtime, 0);
    assert_eq!(usage.lookup(&Text::from("linear_resource")).runtime, 1);
    assert_eq!(usage.lookup(&Text::from("omega_value")).runtime, 3);
}
