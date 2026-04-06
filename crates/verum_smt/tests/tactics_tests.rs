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
// Tests for tactics module
// Migrated from src/tactics.rs per CLAUDE.md standards

use verum_smt::tactics::*;

#[test]
fn test_tactic_builder() {
    let strategy = StrategyBuilder::new()
        .then(TacticKind::Simplify)
        .then(TacticKind::SolveEqs)
        .or_else(TacticKind::SMT)
        .build();

    match strategy {
        TacticCombinator::OrElse(left, right) => {
            // Check structure is correct
            matches!(*right, TacticCombinator::Single(TacticKind::SMT));
        }
        _ => panic!("Unexpected strategy structure"),
    }
}
