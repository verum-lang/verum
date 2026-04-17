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
//! Tests for Verification Profiler

use std::path::PathBuf;
use std::time::Duration;
use verum_compiler::verification_profiler::{
    CacheStatistics, FileLocation, SmtSolver, VerificationProfiler,
};
use verum_common::Text;
use verum_smt::{ProofResult, VerificationCost, VerificationError, VerifyMode};

fn cost(name: &str, duration: Duration, succeeded: bool) -> VerificationCost {
    VerificationCost::new(name.to_string().into(), duration, succeeded)
}

#[test]
fn test_profiler_creation() {
    let profiler = VerificationProfiler::new();
    let report = profiler.generate_report();

    assert_eq!(report.function_count, 0);
    assert_eq!(report.slow_verifications.len(), 0);
}

#[test]
fn test_profiler_with_fast_function() {
    let mut profiler = VerificationProfiler::new();

    // Profile a fast function (< 5s)
    let result = profiler.profile_function(
        "fast_function",
        FileLocation::new(PathBuf::from("test.vr"), 10, 5),
        VerifyMode::Auto,
        || {
            // Simulate fast verification
            std::thread::sleep(Duration::from_millis(100));
            Ok(ProofResult::new(
                cost("fast_function", Duration::from_millis(100), true).with_checks(1),
            ))
        },
    );

    assert!(result.is_ok());

    let report = profiler.generate_report();
    assert_eq!(report.function_count, 1);
    assert_eq!(report.slow_verifications.len(), 0); // Not slow (< 5s)
}

#[test]
fn test_profiler_with_slow_function() {
    let mut profiler = VerificationProfiler::new();

    // Profile a slow function (> 5s)
    let result = profiler.profile_function(
        "slow_function",
        FileLocation::new(PathBuf::from("test.vr"), 20, 10),
        VerifyMode::Proof,
        || {
            // Simulate slow verification
            Ok(ProofResult::new(
                cost("slow_function", Duration::from_secs(6), true).with_checks(10),
            ))
        },
    );

    assert!(result.is_ok());

    let report = profiler.generate_report();
    assert_eq!(report.function_count, 1);
    assert_eq!(report.slow_verifications.len(), 1); // Slow (> 5s)
    assert!(
        report.slow_verifications[0]
            .function_name
            .as_str()
            .contains("slow")
    );
}

#[test]
fn test_profiler_with_timeout() {
    let mut profiler = VerificationProfiler::new();

    // Profile a function that times out
    let result = profiler.profile_function(
        "timeout_function",
        FileLocation::unknown(),
        VerifyMode::Proof,
        || {
            Err(VerificationError::Timeout {
                constraint: Text::from("test_constraint"),
                timeout: Duration::from_secs(30),
                cost: cost("timeout_function", Duration::from_secs(30), false).with_timeout(),
            })
        },
    );

    assert!(result.is_err());

    let report = profiler.generate_report();
    assert_eq!(report.function_count, 1);
}

#[test]
fn test_cache_statistics() {
    let mut stats = CacheStatistics::default();
    assert_eq!(stats.hit_rate(), 0.0);

    stats.hits = 80;
    stats.misses = 20;

    assert_eq!(stats.total_requests(), 100);
    assert_eq!(stats.hit_rate(), 0.8);
}

#[test]
fn test_profiler_recommendations() {
    let mut profiler = VerificationProfiler::new();

    // Add multiple slow functions
    for i in 0..3 {
        let func_name = format!("slow_func_{}", i);
        let _ = profiler.profile_function(
            &func_name,
            FileLocation::unknown(),
            VerifyMode::Proof,
            move || {
                Ok(ProofResult::new(
                    cost("slow_func", Duration::from_secs(7), true).with_checks(1),
                ))
            },
        );
    }

    let report = profiler.generate_report();
    assert!(!report.recommendations.is_empty());
}

#[test]
fn test_profiler_json_export() {
    let mut profiler = VerificationProfiler::new();

    let _ = profiler.profile_function(
        "test_function",
        FileLocation::new(PathBuf::from("example.vr"), 5, 1),
        VerifyMode::Auto,
        || {
            Ok(ProofResult::new(
                cost("test_function", Duration::from_secs(6), true).with_checks(3),
            ))
        },
    );

    // export_json returns a String; use export_json_value for structured access
    let json_str = profiler.export_json();
    let json: serde_json::Value = serde_json::from_str(&json_str).expect("valid JSON");
    assert!(json.is_object());
    assert!(json.get("slow_verifications").is_some());
    assert!(json.get("cache_stats").is_some());
    assert!(json.get("summary").is_some());

    // Also verify export_json_value works directly
    let json_value = profiler.export_json_value();
    assert!(json_value.is_object());
    assert!(json_value.get("all_entries").is_some());
    assert!(json_value["summary"]["functions_above_threshold"].is_u64());
}

#[test]
fn test_smt_solver_display() {
    assert_eq!(SmtSolver::Z3.as_str(), "Z3");
    assert_eq!(SmtSolver::CVC5.as_str(), "CVC5");
}
