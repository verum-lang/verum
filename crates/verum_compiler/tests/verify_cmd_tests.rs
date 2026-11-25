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
//! Migrated from src/verify_cmd.rs to comply with AGENTS.md test organization.

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
fn theorem_evidence_survives_aggregation_and_json() {
    use verum_compiler::phases::proof_verification::ProofEvidence;
    let mut report = VerificationReport::new();
    for (name, evidence) in [
        ("open", ProofEvidence::Assumed),
        ("unfinished", ProofEvidence::Admitted),
        ("checked", ProofEvidence::TacticDischarge),
    ] {
        report.add_result(
            name.into(),
            VerificationResult::Theorem {
                evidence,
                elapsed: Duration::ZERO,
            },
        );
    }
    let json = report.to_json();
    assert_eq!(json.proved, 0);
    assert_eq!(
        (json.tactic_checked, json.admitted, json.assumed),
        (1, 1, 1)
    );
    assert_eq!(report.failed_names(), ["open", "unfinished"]);
    let statuses: Vec<_> = json.results.iter().map(|r| r.status.as_str()).collect();
    assert_eq!(statuses, ["assumed", "admitted", "tactic_checked"]);
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

#[test]
fn command_reports_the_evidence_of_each_real_source_declaration() {
    use verum_compiler::verify_cmd::VerifyCommand;
    use verum_compiler::{CompilerOptions, Session};

    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("evidence.vr");
    std::fs::write(
        &input,
        "@no_implicit_prelude module test.evidence;
        axiom premise(n:Int) requires n >= 0 ensures n >= 0;
        theorem assumed(): false;
        theorem incomplete(): true proof by admit;
        theorem checked(b:Bool) requires b : b proof by assumption;
        fn plain(n:Int)->Int { n }",
    )
    .unwrap();
    let mut session = Session::new(CompilerOptions {
        input,
        ..Default::default()
    });
    let report = VerifyCommand::new(&mut session)
        .run_to_report(None)
        .unwrap()
        .to_json();
    let statuses: std::collections::BTreeMap<_, _> = report
        .results
        .iter()
        .map(|result| (result.function.as_str(), result.status.as_str()))
        .collect();
    assert_eq!(statuses["axiom premise"], "assumed");
    assert_eq!(statuses["theorem assumed"], "assumed");
    assert_eq!(statuses["theorem incomplete"], "admitted");
    assert_eq!(statuses["theorem checked"], "tactic_checked");
    assert_eq!(statuses["plain"], "no_obligations");
    assert_eq!(
        (
            report.proved,
            report.tactic_checked,
            report.assumed,
            report.admitted,
            report.no_obligations
        ),
        (0, 1, 2, 1, 1)
    );
    assert!(
        VerifyCommand::new(&mut session)
            .run(None)
            .unwrap_err()
            .to_string()
            .contains("without a completed proof")
    );
}
