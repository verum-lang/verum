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
// Unit tests for cost.rs
//
// Migrated from src/cost.rs to comply with CLAUDE.md test organization.

use std::time::Duration;
use verum_smt::cost::*;

#[test]
fn test_cost_tracker() {
    let mut tracker = CostTracker::new();

    let cost1 = VerificationCost::new("test1".into(), Duration::from_secs(2), true);
    let cost2 = VerificationCost::new("test2".into(), Duration::from_secs(8), true);

    tracker.record(cost1);
    tracker.record(cost2);

    assert_eq!(tracker.costs().len(), 2);
    assert_eq!(tracker.total_time(), Duration::from_secs(10));
    assert_eq!(tracker.avg_time(), Duration::from_secs(5));
}

#[test]
fn test_slow_verifications() {
    let mut tracker = CostTracker::with_threshold(Duration::from_secs(3));

    tracker.record(VerificationCost::new(
        "fast".into(),
        Duration::from_secs(1),
        true,
    ));
    tracker.record(VerificationCost::new(
        "slow".into(),
        Duration::from_secs(5),
        true,
    ));

    let slow = tracker.slow_verifications();
    assert_eq!(slow.len(), 1);
    assert_eq!(slow[0].location, "slow");
    assert!(tracker.should_suggest_runtime());
}

#[test]
fn test_cost_measurement() {
    let measurement = CostMeasurement::start("test_function")
        .with_checks(3)
        .with_complexity(50);

    std::thread::sleep(Duration::from_millis(10));

    let cost = measurement.finish(true);
    assert_eq!(cost.location, "test_function");
    assert!(cost.succeeded);
    assert_eq!(cost.num_checks, 3);
    assert_eq!(cost.complexity, 50);
    assert!(cost.duration >= Duration::from_millis(10));
}

#[test]
fn test_cost_report_format() {
    let mut tracker = CostTracker::new();

    tracker.record(VerificationCost::new(
        "func1".into(),
        Duration::from_secs(2),
        true,
    ));
    tracker.record(VerificationCost::new(
        "func2".into(),
        Duration::from_secs(6),
        true,
    ));

    let report = tracker.report();
    let formatted = report.format();

    assert!(formatted.contains("Verification Summary"));
    assert!(formatted.contains("func2"));
    assert!(formatted.contains("@verify(runtime)"));
}

#[test]
fn test_format_success() {
    let cost = VerificationCost::new("test".into(), Duration::from_millis(500), true);
    let msg = format_success("my_function", &cost);
    assert!(msg.contains("✓"));
    assert!(msg.contains("my_function"));
}

#[test]
fn test_format_failure() {
    let cost = VerificationCost::new("test".into(), Duration::from_secs(3), false).with_timeout();

    let msg = format_failure("my_function", "x > 0", &cost);
    assert!(msg.contains("✗"));
    assert!(msg.contains("x > 0"));
    assert!(msg.contains("timeout"));
}
