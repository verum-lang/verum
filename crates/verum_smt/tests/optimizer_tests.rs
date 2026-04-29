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
// Tests for optimizer module
// Migrated from src/optimizer.rs per CLAUDE.md standards

use verum_smt::optimizer::*;
use z3::ast::Bool;

#[test]
fn test_maxsat_solver() {
    let mut solver = MaxSATSolver::new();

    // Add some hard and soft clauses
    let x = Bool::new_const("x");
    let y = Bool::new_const("y");

    solver.add_hard(Bool::or(&[&x, &y]));
    solver.add_soft(x.clone(), 1);
    solver.add_soft(y.clone(), 2);

    let result = solver.solve();
    assert!(result.sat);
}

// =============================================================================
// OptimizerConfig.method wiring tests
// =============================================================================
//
// Pin: `OptimizerConfig.method` reaches Z3's `:opt.priority`
// parameter through `Z3Optimizer::new`. Pre-fix the field landed
// on the optimizer but no code path consulted it, so callers
// that set `method = Pareto` for multi-objective frontier
// exploration silently got the lex default.

#[test]
fn optimizer_config_method_default_is_lexicographic() {
    let cfg = OptimizerConfig::default();
    assert!(matches!(cfg.method, OptimizationMethod::Lexicographic));
}

#[test]
fn optimizer_method_round_trips_for_all_variants() {
    // Pin: every documented method variant is constructible and
    // round-trips through the config without coupling. This
    // catches a future regression where someone removes a
    // variant or changes the discriminant ordering.
    for method in [
        OptimizationMethod::Lexicographic,
        OptimizationMethod::Pareto,
        OptimizationMethod::Independent,
        OptimizationMethod::Box,
    ] {
        let cfg = OptimizerConfig {
            method: method.clone(),
            ..OptimizerConfig::default()
        };
        assert_eq!(cfg.method, method);
    }
}

#[test]
fn optimizer_constructor_accepts_all_methods_without_panic() {
    // Pin: Z3Optimizer::new wires `method` into Z3's
    // :opt.priority via Params. The visible failure mode if
    // priority_str were set to something Z3 rejects (e.g. an
    // unknown symbol) would be a panic at set_params time. This
    // test exercises the wire-up for every variant — if a
    // future change introduces a typo or a Z3-incompatible
    // priority string, it surfaces here, not in production
    // optimizer construction.
    for method in [
        OptimizationMethod::Lexicographic,
        OptimizationMethod::Pareto,
        OptimizationMethod::Independent,
        OptimizationMethod::Box,
    ] {
        let cfg = OptimizerConfig {
            method: method.clone(),
            ..OptimizerConfig::default()
        };
        let _opt = Z3Optimizer::new(cfg);
        // If we got here, set_params accepted the priority.
    }
}
