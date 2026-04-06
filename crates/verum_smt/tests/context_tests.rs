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
// Unit tests for context.rs
//
// Migrated from src/context.rs to comply with CLAUDE.md test organization.

use std::time::Duration;
use verum_smt::context::*;

#[test]
fn test_context_creation() {
    let _ctx = Context::new();
}

#[test]
fn test_context_with_config() {
    let config = ContextConfig::fast();
    let _ctx = Context::with_config(config);
}

#[test]
fn test_solver_creation() {
    let ctx = Context::new();
    let _solver = ctx.solver();
}

#[test]
fn test_config_builder() {
    let config = ContextConfig::default()
        .with_timeout(Duration::from_secs(10))
        .with_memory_limit(512)
        .with_models()
        .with_seed(42);

    assert_eq!(config.timeout, Some(Duration::from_secs(10)));
    assert_eq!(config.memory_limit_mb, Some(512));
    assert!(config.model_generation);
    assert_eq!(config.random_seed, Some(42));
}

#[test]
fn test_solver_stats() {
    let mut stats = SolverStats::new();

    stats.record_sat(100);
    stats.record_unsat(200);
    stats.record_unknown(150);

    assert_eq!(stats.num_checks, 3);
    assert_eq!(stats.num_sat, 1);
    assert_eq!(stats.num_unsat, 1);
    assert_eq!(stats.num_unknown, 1);
    assert_eq!(stats.time_ms, 450);
    assert_eq!(stats.avg_time_ms(), 150.0);
}

#[test]
fn test_success_rate() {
    let mut stats = SolverStats::new();
    stats.record_sat(100);
    stats.record_unsat(100);
    stats.record_unknown(100);

    assert_eq!(stats.success_rate(), 2.0 / 3.0);
}
