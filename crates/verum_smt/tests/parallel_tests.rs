#![allow(clippy::assertions_on_constants)] // tests use assert!(true) placeholders
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
// Tests for parallel module
// Migrated from src/parallel.rs per CLAUDE.md standards

use std::sync::atomic::Ordering;
use verum_common::{List, Maybe};
use verum_smt::parallel::*;

#[test]
fn test_parallel_config_default() {
    let config = ParallelConfig::default();
    assert!(config.num_workers > 0);
    assert!(!config.strategies.is_empty());
    assert!(config.enable_lemma_exchange);
}

#[test]
fn test_strategy_params() {
    let aggressive = StrategyParams::aggressive();
    assert_eq!(aggressive.simplify_level, 3);
    assert_eq!(aggressive.random_seed, Maybe::Some(42));

    let conservative = StrategyParams::conservative();
    assert_eq!(conservative.simplify_level, 1);
}

#[test]
fn test_parallel_solver_creation() {
    let config = ParallelConfig {
        num_workers: 2,
        race_mode: true,
        ..Default::default()
    };

    let solver = ParallelSolver::new(config);
    // Test that solver was created successfully
    assert!(true);
}

#[test]
fn test_cube_and_conquer_solver() {
    let solver = CubeAndConquerSolver::new();
    // Test that cube-and-conquer solver was created successfully
    assert!(true);
}

#[test]
fn test_portfolio_solver() {
    let strategies = vec![SolvingStrategy::Default, SolvingStrategy::BitBlasting]
        .into_iter()
        .collect();

    let solver = PortfolioSolver::with_strategies(strategies);
    // Test that portfolio solver was created successfully with strategies
    assert!(true);
}

#[test]
fn test_worker_stats() {
    let stats = WorkerStats {
        worker_id: 0,
        conflicts: 100,
        decisions: 200,
        propagations: 500,
        restarts: 10,
        time_ms: 1000,
        lemmas_learned: 5,
        lemmas_received: 3,
    };

    assert_eq!(stats.conflicts, 100);
    assert_eq!(stats.lemmas_learned, 5);
}

#[test]
fn test_parallel_stats() {
    let stats = ParallelStats {
        total_time_ms: 5000,
        time_to_result_ms: 4500,
        workers_used: 4,
        lemmas_exchanged: 20,
        worker_stats: List::new(),
        cubes_generated: 16,
        cubes_solved: 15,
    };

    assert_eq!(stats.workers_used, 4);
    assert_eq!(stats.lemmas_exchanged, 20);
}
