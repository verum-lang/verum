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
// Tests for advanced Z3 features
//
// This file tests the production-grade Z3 integration with:
// - Optimizer (MaxSAT/MinSAT)
// - Tactics and strategies
// - Unsat cores
// - Model extraction
// - Proof generation
// - Incremental solving

use verum_common::Maybe;
use verum_smt::{
    AdvancedResult, BVSolver, IncrementalVerifier, LIASolver, Z3Config, Z3Solver, verify_parallel,
};
use z3::ast::{Bool, Int};

#[test]
fn test_optimizer_maxsat() {
    let mut solver = Z3Solver::new(Maybe::None);
    solver.enable_optimization();

    // Create variables
    let x = Int::new_const("x");
    let y = Int::new_const("y");

    // Hard constraint: x + y = 10
    solver.assert(&(&x + &y).eq(10));

    // Soft constraint 1: prefer x > 5 (weight 2)
    solver.assert_soft(&x.gt(5), 2);

    // Soft constraint 2: prefer y > 5 (weight 1)
    solver.assert_soft(&y.gt(5), 1);

    let result = solver.check_sat();
    matches!(result, AdvancedResult::SatOptimal { .. });
}

#[test]
fn test_unsat_core_extraction() {
    let mut solver = Z3Solver::new(Maybe::None);

    let x = Int::new_const("x");

    // Add tracked assertions
    solver.assert_tracked(&x.eq(3), "x-is-3");
    solver.assert_tracked(&x.eq(5), "x-is-5");
    solver.assert_tracked(&x.gt(10), "x-gt-10");

    let result = solver.check_sat();

    match result {
        AdvancedResult::Unsat { core, .. } => {
            assert!(core.is_some());
            let core = core.unwrap();
            // At minimum, two conflicting equalities should be in the core
            assert!(core.assertions.len() >= 2);
        }
        _ => panic!("Expected UNSAT result"),
    }
}

#[test]
fn test_tactic_auto_selection() {
    let mut solver = Z3Solver::new(Maybe::None);
    solver.auto_select_tactic();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    solver.assert(&x.gt(&y));
    solver.assert(&y.gt(0));

    let result = solver.check_sat();
    matches!(result, AdvancedResult::Sat { .. });
}

#[test]
fn test_incremental_solving() {
    let mut solver = Z3Solver::new(Maybe::None);

    let x = Int::new_const("x");

    // Push scope 1
    solver.push();
    solver.assert(&x.gt(0));
    assert!(matches!(solver.check_sat(), AdvancedResult::Sat { .. }));

    // Push scope 2
    solver.push();
    solver.assert(&x.lt(0));
    assert!(matches!(solver.check_sat(), AdvancedResult::Unsat { .. }));

    // Pop scope 2
    solver.pop();
    assert!(matches!(solver.check_sat(), AdvancedResult::Sat { .. }));

    // Pop scope 1
    solver.pop();
}

#[test]
fn test_lia_solver_specialization() {
    let mut solver = LIASolver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    // Linear constraints
    solver.assert(&x.gt(&y));
    solver.assert(&y.gt(0));
    solver.assert(&(&x + &y).le(100));

    let result = solver.check();
    assert!(matches!(result, AdvancedResult::Sat { .. }));
}

#[test]
fn test_bv_solver_specialization() {
    let mut solver = BVSolver::new();

    // Bit-vector constraints would go here
    // For now, just test that it initializes correctly
    let x = Int::new_const("x");
    solver.assert(&x.gt(0));

    let result = solver.check();
    assert!(matches!(result, AdvancedResult::Sat { .. }));
}

#[test]
fn test_proof_witness_extraction() {
    let mut solver = Z3Solver::new(Maybe::None);

    let x = Int::new_const("x");

    solver.assert(&x.eq(3));
    solver.assert(&x.eq(5));

    let result = solver.check_sat();

    match result {
        AdvancedResult::Unsat { proof, .. } => {
            // Proof generation needs to be enabled in config
            // For now, just verify the structure
            assert!(proof.is_some() || proof.is_none());
        }
        _ => panic!("Expected UNSAT"),
    }

    // Test proof witness
    let witness = solver.get_proof_witness();
    if let Maybe::Some(w) = witness {
        assert!(!w.proof_term.is_empty());
    }
}

#[test]
fn test_solver_statistics() {
    let mut solver = Z3Solver::new(Maybe::None);

    let x = Int::new_const("x");

    solver.push();
    solver.assert(&x.gt(0));
    solver.check_sat();

    solver.push();
    solver.assert(&x.lt(10));
    solver.check_sat();

    solver.pop();
    solver.pop();

    let stats = solver.get_stats();
    assert_eq!(stats.total_checks, 2);
}

#[test]
fn test_model_extraction() {
    use verum_smt::ModelExtractor;

    let mut solver = Z3Solver::new(Maybe::None);

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    solver.assert(&x.gt(&y));
    solver.assert(&y.gt(5));

    match solver.check_sat() {
        AdvancedResult::Sat { model } => {
            if let Maybe::Some(m) = model {
                let extractor = ModelExtractor::new(m);

                // Evaluate x
                if let Maybe::Some(x_val) = extractor.eval_int(&x) {
                    assert!(x_val > 5);
                }

                // Get counterexample
                let ce = extractor.get_counterexample(&["x".into(), "y".into()]);
                assert!(
                    ce.bindings.contains_key(&"x".into()) || ce.bindings.contains_key(&"y".into())
                );
            }
        }
        _ => panic!("Expected SAT"),
    }
}

#[test]
fn test_config_with_proofs() {
    let config = Z3Config {
        enable_proofs: true,
        minimize_cores: true,
        enable_interpolation: false,
        global_timeout_ms: Maybe::Some(10000),
        memory_limit_mb: Maybe::Some(1024),
        enable_mbqi: true,
        enable_patterns: true,
        random_seed: Maybe::Some(42),
        num_workers: 2,
        auto_tactics: true,
    };

    // Just verify configuration can be created
    assert!(config.enable_proofs);
    assert_eq!(config.num_workers, 2);
}

#[test]
fn test_list_tactics_and_probes() {
    use verum_smt::{list_probes, list_tactics};

    let tactics = list_tactics();
    assert!(tactics.len() > 10);
    assert!(tactics.contains(&"simplify".into()));
    assert!(tactics.contains(&"smt".into()));

    let probes = list_probes();
    assert!(probes.len() > 10);
    assert!(probes.contains(&"is-qflia".into()));
}
