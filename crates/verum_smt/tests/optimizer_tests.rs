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
