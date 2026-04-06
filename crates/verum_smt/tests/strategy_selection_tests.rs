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
// Tests for strategy_selection module
// Migrated from src/strategy_selection.rs per CLAUDE.md standards

use verum_smt::strategy_selection::*;

use std::time::Duration;
use z3::ast::{Bool, Int};

#[test]
fn test_strategy_selector_creation() {
    let selector = StrategySelector::new();
    assert!(selector.enable_auto_selection);
    assert_eq!(selector.fallback_tactic, TacticKind::SMT);
}

#[test]
fn test_empty_constraints() {
    let selector = StrategySelector::new();
    let constraints: Vec<Bool> = vec![];
    let tactic = selector.select_tactic(&constraints);
    // Should return fallback tactic
    // (we can't easily test tactic equality, so just ensure it doesn't panic)
    let _ = tactic;
}

#[test]
fn test_timeout_estimation() {
    let selector = StrategySelector::new();

    // Empty constraints - should be fast
    let empty: Vec<Bool> = vec![];
    let timeout = selector.estimate_timeout(&empty);
    assert_eq!(timeout, Duration::from_secs(1));
}

#[test]
fn test_parallel_strategies_empty() {
    let selector = StrategySelector::new();
    let empty: Vec<Bool> = vec![];
    let strategies = selector.get_parallel_strategies(&empty);
    assert_eq!(strategies.len(), 1);
    assert_eq!(strategies[0], TacticKind::SMT);
}

#[test]
fn test_tactic_descriptions() {
    assert_eq!(TacticKind::SMT.description(), "Default SMT solver");
    assert_eq!(
        TacticKind::Fast.description(),
        "Fast tactic for small problems"
    );
    assert_eq!(
        TacticKind::LinearArithmetic.description(),
        "Linear integer arithmetic"
    );
}

#[test]
fn test_complexity_thresholds() {
    let default = ComplexityThresholds::default();
    assert_eq!(default.small_problem_size, 100.0);

    let conservative = ComplexityThresholds::conservative();
    assert!(conservative.small_problem_size < default.small_problem_size);

    let aggressive = ComplexityThresholds::aggressive();
    assert!(aggressive.small_problem_size > default.small_problem_size);
}

#[test]
fn test_strategy_stats() {
    let mut stats = StrategyStats::default();
    stats.record_usage(TacticKind::SMT);
    stats.record_usage(TacticKind::SMT);
    stats.record_usage(TacticKind::Fast);

    assert_eq!(stats.tactic_usage.len(), 2);
    assert_eq!(*stats.tactic_usage.get("Default SMT solver").unwrap(), 2);
    assert_eq!(
        *stats
            .tactic_usage
            .get("Fast tactic for small problems")
            .unwrap(),
        1
    );

    let most_used = stats.most_used_tactic();
    assert!(most_used.is_some());
    let (name, count) = most_used.unwrap();
    assert_eq!(name, "Default SMT solver");
    assert_eq!(count, 2);
}
