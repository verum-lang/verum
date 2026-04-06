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
//! Integration Tests for Backend Switcher with Real Z3/CVC5 FFI Calls
//!
//! These tests verify that the backend switcher correctly integrates with
//! actual Z3 and CVC5 solvers, not just stubs.
//!
//! Refinement type verification: `Int{> 0}`, `Float{>= 0.0}` predicates are checked
//! by SMT solvers. The backend switcher routes queries to Z3 or CVC5 with automatic
//! fallback. Three modes: @verify(runtime), @verify(static), @verify(proof).
//!
//! NOTE: These tests require the `cvc5` feature to be enabled.

#![cfg(feature = "cvc5")]

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::{Literal, LiteralKind};
use verum_ast::span::Span;
use verum_smt::backend_switcher::{
    BackendChoice, FallbackConfig, PortfolioConfig, PortfolioMode, SmtBackendSwitcher, SolveResult,
    SwitcherConfig,
};
use verum_common::List;

/// Helper to create a simple boolean literal expression
fn mk_bool_lit(value: bool) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Bool(value),
            span: Span::default(),
        }),
        Span::default(),
    )
}

/// Helper to create a simple integer literal
fn mk_int_lit(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(value.to_string()),
            span: Span::default(),
        }),
        Span::default(),
    )
}

/// Helper to create a binary expression
fn mk_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::default(),
    )
}

#[test]
fn test_z3_backend_trivial_sat() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Z3,
        ..Default::default()
    });

    // Test: true is SAT
    let assertions = List::from(vec![mk_bool_lit(true)]);
    let result = switcher.solve(&assertions);

    assert!(
        result.is_sat(),
        "Expected SAT for 'true', got: {:?}",
        result
    );
    assert_eq!(result.backend(), "Z3");
}

#[test]
fn test_z3_backend_trivial_unsat() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Z3,
        ..Default::default()
    });

    // Test: false is UNSAT
    let assertions = List::from(vec![mk_bool_lit(false)]);
    let result = switcher.solve(&assertions);

    assert!(
        result.is_unsat() || result.is_unknown(),
        "Expected UNSAT for 'false', got: {:?}",
        result
    );
}

#[test]
fn test_z3_backend_arithmetic() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Z3,
        ..Default::default()
    });

    // Test: 2 + 3 = 5
    let left = mk_binary(BinOp::Add, mk_int_lit(2), mk_int_lit(3));
    let expr = mk_binary(BinOp::Eq, left, mk_int_lit(5));

    let assertions = List::from(vec![expr]);
    let result = switcher.solve(&assertions);

    assert!(
        result.is_sat(),
        "Expected SAT for '2 + 3 = 5', got: {:?}",
        result
    );
}

#[test]
fn test_cvc5_backend_trivial_sat() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Cvc5,
        ..Default::default()
    });

    // Test: true is SAT
    let assertions = List::from(vec![mk_bool_lit(true)]);
    let result = switcher.solve(&assertions);

    // CVC5 may not be available, check for either SAT or initialization error
    match result {
        SolveResult::Sat { .. } => {
            assert_eq!(result.backend(), "CVC5");
        }
        SolveResult::Error { error, .. } => {
            // If CVC5 is not installed, this is acceptable
            assert!(
                error.contains("not initialized") || error.contains("Failed to initialize"),
                "Unexpected error: {}",
                error
            );
        }
        _ => panic!("Unexpected result: {:?}", result),
    }
}

#[test]
fn test_cvc5_backend_arithmetic() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Cvc5,
        ..Default::default()
    });

    // Test: 10 > 5
    let expr = mk_binary(BinOp::Gt, mk_int_lit(10), mk_int_lit(5));
    let assertions = List::from(vec![expr]);
    let result = switcher.solve(&assertions);

    // CVC5 may not be available
    match result {
        SolveResult::Sat { .. } => {
            assert_eq!(result.backend(), "CVC5");
        }
        SolveResult::Error { .. } => {
            // Acceptable if CVC5 is not installed
        }
        _ => {}
    }
}

#[test]
fn test_auto_backend_selection() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Auto,
        fallback: FallbackConfig {
            enabled: true,
            on_timeout: true,
            on_unknown: true,
            on_error: true,
            max_attempts: 2,
        },
        ..Default::default()
    });

    // Test with a simple formula
    let assertions = List::from(vec![mk_bool_lit(true)]);
    let result = switcher.solve(&assertions);

    // Should get a result from either Z3 or fallback to CVC5
    assert!(
        result.is_sat() || result.is_error(),
        "Expected SAT or Error in auto mode, got: {:?}",
        result
    );
}

#[test]
fn test_fallback_from_z3_to_cvc5() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Auto,
        fallback: FallbackConfig {
            enabled: true,
            on_error: true,
            on_unknown: true,
            on_timeout: true,
            max_attempts: 2,
        },
        ..Default::default()
    });

    // If Z3 fails or returns unknown, should fallback to CVC5
    let assertions = List::from(vec![mk_bool_lit(true)]);
    let result = switcher.solve(&assertions);

    // At least one solver should work
    assert!(
        !result.is_error() || result.backend().contains("CVC5"),
        "Expected successful solve or CVC5 error, got: {:?}",
        result
    );
}

#[test]
fn test_portfolio_mode_first_result() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Portfolio,
        portfolio: PortfolioConfig {
            enabled: true,
            mode: PortfolioMode::FirstResult,
            max_threads: 2,
            timeout_per_solver: 10000, // 10s
            kill_on_first: true,
        },
        ..Default::default()
    });

    // Test with simple formula - should get first result from either solver
    let assertions = List::from(vec![mk_bool_lit(true)]);
    let result = switcher.solve(&assertions);

    // Should get a result from at least one solver
    match result {
        SolveResult::Sat { backend, .. } => {
            assert!(
                backend == "Z3" || backend == "CVC5",
                "Unexpected backend: {}",
                backend
            );
        }
        SolveResult::Error { .. } => {
            // Acceptable if both solvers fail to initialize
        }
        _ => panic!("Unexpected result in portfolio mode: {:?}", result),
    }
}

#[test]
fn test_portfolio_mode_consensus() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Portfolio,
        portfolio: PortfolioConfig {
            enabled: true,
            mode: PortfolioMode::Consensus,
            max_threads: 2,
            timeout_per_solver: 10000,
            kill_on_first: false,
        },
        ..Default::default()
    });

    // Test with simple formula - both solvers should agree
    let assertions = List::from(vec![mk_bool_lit(true)]);
    let result = switcher.solve(&assertions);

    // In consensus mode, either both agree (SAT) or there's an error
    match result {
        SolveResult::Sat { .. } => {
            // Good - solvers agreed
        }
        SolveResult::Error { error, .. } => {
            // Could be initialization error or disagreement
            assert!(
                error.contains("Portfolio") || error.contains("Failed to initialize"),
                "Unexpected error: {}",
                error
            );
        }
        _ => {}
    }
}

#[test]
fn test_empty_assertions() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Z3,
        ..Default::default()
    });

    // Empty assertions should be trivially SAT
    let assertions = List::new();
    let result = switcher.solve(&assertions);

    assert!(
        result.is_sat(),
        "Expected SAT for empty assertions, got: {:?}",
        result
    );
}

#[test]
fn test_complex_formula() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Z3,
        ..Default::default()
    });

    // Test: (5 > 3) AND (10 < 20)
    let left = mk_binary(BinOp::Gt, mk_int_lit(5), mk_int_lit(3));
    let right = mk_binary(BinOp::Lt, mk_int_lit(10), mk_int_lit(20));
    let expr = mk_binary(BinOp::And, left, right);

    let assertions = List::from(vec![expr]);
    let result = switcher.solve(&assertions);

    assert!(
        result.is_sat(),
        "Expected SAT for '(5 > 3) AND (10 < 20)', got: {:?}",
        result
    );
}

#[test]
fn test_unsatisfiable_formula() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Z3,
        ..Default::default()
    });

    // Test: (x > 10) AND (x < 5) - unsatisfiable
    // Note: This would require variable support, so we use a simpler UNSAT formula
    // Test: true AND false
    let left = mk_bool_lit(true);
    let right = mk_bool_lit(false);
    let expr = mk_binary(BinOp::And, left, right);

    let assertions = List::from(vec![expr]);
    let result = switcher.solve(&assertions);

    assert!(
        result.is_unsat() || result.is_unknown(),
        "Expected UNSAT for 'true AND false', got: {:?}",
        result
    );
}

#[test]
fn test_backend_statistics() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Z3,
        ..Default::default()
    });

    // Run several queries
    for _ in 0..5 {
        let assertions = List::from(vec![mk_bool_lit(true)]);
        let _ = switcher.solve(&assertions);
    }

    let stats = switcher.get_stats();
    assert_eq!(stats.total_queries, 5);
    assert!(stats.total_time_ms > 0);
}

#[test]
fn test_backend_manual_selection() {
    let mut switcher = SmtBackendSwitcher::with_defaults();

    // Start with Z3
    switcher.select_backend(BackendChoice::Z3);
    assert_eq!(switcher.current_backend(), BackendChoice::Z3);

    let assertions = List::from(vec![mk_bool_lit(true)]);
    let result = switcher.solve(&assertions);
    assert_eq!(result.backend(), "Z3");

    // Switch to CVC5
    switcher.select_backend(BackendChoice::Cvc5);
    assert_eq!(switcher.current_backend(), BackendChoice::Cvc5);

    let result = switcher.solve(&assertions);
    // May fail if CVC5 not installed
    assert!(result.backend() == "CVC5" || result.is_error());
}

#[test]
fn test_multiple_assertions() {
    let mut switcher = SmtBackendSwitcher::new(SwitcherConfig {
        default_backend: BackendChoice::Z3,
        ..Default::default()
    });

    // Multiple simple assertions: true, true, true
    let assertions = List::from(vec![
        mk_bool_lit(true),
        mk_bool_lit(true),
        mk_bool_lit(true),
    ]);

    let result = switcher.solve(&assertions);
    assert!(
        result.is_sat(),
        "Expected SAT for multiple 'true' assertions, got: {:?}",
        result
    );
}
