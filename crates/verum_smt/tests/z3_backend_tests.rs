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
// Tests for z3_backend module
// Migrated from src/z3_backend.rs per CLAUDE.md standards

use verum_common::Maybe;
use verum_smt::z3_backend::*;
use z3::ast::Int;

#[test]
fn test_basic_solver() {
    let mut solver = Z3Solver::new(Maybe::None);

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    solver.assert(&x.gt(&y));

    let result = solver.check_sat();
    matches!(result, AdvancedResult::Sat { .. });
}

#[test]
fn test_unsat_core() {
    let mut solver = Z3Solver::new(Maybe::None);

    let x = Int::new_const("x");

    solver.assert_tracked(&x.eq(3), "x-is-3");
    solver.assert_tracked(&x.eq(5), "x-is-5");

    let result = solver.check_sat();

    match result {
        AdvancedResult::Unsat { core, .. } => {
            assert!(core.is_some());
            let core = core.unwrap();
            assert_eq!(core.assertions.len(), 2);
        }
        _ => panic!("Expected UNSAT"),
    }
}

#[test]
fn test_incremental_solving() {
    let mut solver = Z3Solver::new(Maybe::None);

    let x = Int::new_const("x");

    solver.push();
    solver.assert(&x.gt(0));
    assert!(matches!(solver.check_sat(), AdvancedResult::Sat { .. }));

    solver.push();
    solver.assert(&x.lt(0));
    assert!(matches!(solver.check_sat(), AdvancedResult::Unsat { .. }));

    solver.pop();
    assert!(matches!(solver.check_sat(), AdvancedResult::Sat { .. }));
}

#[test]
fn test_tactic_selection() {
    let mut solver = Z3Solver::new(Maybe::None);
    solver.auto_select_tactic();

    let x = Int::new_const("x");
    solver.assert(&x.gt(0));

    let result = solver.check_sat();
    assert!(matches!(result, AdvancedResult::Sat { .. }));
}

#[test]
fn test_lia_solver() {
    let mut solver = LIASolver::new();

    let x = Int::new_const("x");
    let y = Int::new_const("y");

    solver.assert(&x.gt(&y));
    solver.assert(&y.gt(0));
    solver.assert(&(&x + &y).eq(10));

    let result = solver.check();
    assert!(matches!(result, AdvancedResult::Sat { .. }));
}

#[test]
fn test_list_tactics() {
    let tactics = list_tactics();
    assert!(tactics.contains(&"simplify".into()));
    assert!(tactics.contains(&"smt".into()));
    assert!(tactics.len() > 10);
}

#[test]
fn test_list_probes() {
    let probes = list_probes();
    assert!(probes.contains(&"is-qflia".into()));
    assert!(probes.len() > 10);
}
