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
//! Tests for verify_cmd module
//!
//! Migrated from src/verify_cmd.rs to comply with CLAUDE.md test organization.

use std::time::Duration;
use verum_compiler::verify_cmd::{VerificationReport, VerificationResult};

#[test]
fn test_verification_report_new() {
    let report = VerificationReport::new();
    assert_eq!(report.num_proved(), 0);
    assert_eq!(report.num_failed(), 0);
    assert_eq!(report.num_timeout(), 0);
    assert_eq!(report.num_skipped(), 0);
    assert!(!report.has_failures());
}

#[test]
fn test_verification_report_add_proved() {
    let mut report = VerificationReport::new();
    report.add_result(
        "test_fn".into(),
        VerificationResult::Proved {
            elapsed: Duration::from_millis(100),
        },
    );

    assert_eq!(report.num_proved(), 1);
    assert_eq!(report.num_failed(), 0);
    assert!(!report.has_failures());
}

#[test]
fn test_verification_report_add_failed() {
    let mut report = VerificationReport::new();
    report.add_result(
        "failing_fn".into(),
        VerificationResult::Failed {
            counterexample: Some("x = 0".into()),
            elapsed: Duration::from_millis(50),
        },
    );

    assert_eq!(report.num_proved(), 0);
    assert_eq!(report.num_failed(), 1);
    assert!(report.has_failures());
}

#[test]
fn test_verification_report_add_timeout() {
    let mut report = VerificationReport::new();
    report.add_result(
        "slow_fn".into(),
        VerificationResult::Timeout {
            elapsed: Duration::from_secs(30),
            timeout: Duration::from_secs(30),
        },
    );

    assert_eq!(report.num_timeout(), 1);
    assert_eq!(report.num_failed(), 0);
    assert!(!report.has_failures()); // Timeouts are not failures
}

#[test]
fn test_verification_report_add_skipped() {
    let mut report = VerificationReport::new();
    report.add_result("simple_fn".into(), VerificationResult::Skipped);

    assert_eq!(report.num_skipped(), 1);
    assert_eq!(report.num_proved(), 0);
    assert!(!report.has_failures());
}

#[test]
fn test_verification_report_mixed_results() {
    let mut report = VerificationReport::new();

    // Add multiple results
    report.add_result(
        "proven_fn".into(),
        VerificationResult::Proved {
            elapsed: Duration::from_millis(100),
        },
    );
    report.add_result(
        "failed_fn".into(),
        VerificationResult::Failed {
            counterexample: None,
            elapsed: Duration::from_millis(50),
        },
    );
    report.add_result(
        "timeout_fn".into(),
        VerificationResult::Timeout {
            elapsed: Duration::from_secs(30),
            timeout: Duration::from_secs(30),
        },
    );
    report.add_result("skipped_fn".into(), VerificationResult::Skipped);

    assert_eq!(report.num_proved(), 1);
    assert_eq!(report.num_failed(), 1);
    assert_eq!(report.num_timeout(), 1);
    assert_eq!(report.num_skipped(), 1);
    assert!(report.has_failures());
}

#[test]
fn test_verification_report_total_time() {
    let report = VerificationReport::new();
    // Just verify that total_time() returns a valid Duration
    let _duration = report.total_time();
    // The time should be very small since we just created the report
    assert!(report.total_time().as_secs() < 10);
}

#[test]
fn test_verification_report_to_json() {
    let mut report = VerificationReport::new();
    report.add_result(
        "test_fn".into(),
        VerificationResult::Proved {
            elapsed: Duration::from_millis(100),
        },
    );

    let json = report.to_json();
    assert_eq!(json.total_functions, 1);
    assert_eq!(json.proved, 1);
    assert_eq!(json.failed, 0);
    assert_eq!(json.results.len(), 1);
    assert_eq!(json.results[0].function, "test_fn");
    assert_eq!(json.results[0].status, "proved");
}
