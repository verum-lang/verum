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
// Tests for security module
// Migrated from src/security.rs per CLAUDE.md standards

use verum_cli::security::*;

#[test]
fn test_scanner_creation() {
    let scanner = SecurityScanner::new();
    assert_eq!(scanner.audit_log.len(), 0);
}

#[test]
fn test_severity_score() {
    let scanner = SecurityScanner::new();
    assert_eq!(scanner.severity_score(&Severity::Low), 1.0);
    assert_eq!(scanner.severity_score(&Severity::Medium), 5.0);
    assert_eq!(scanner.severity_score(&Severity::High), 10.0);
    assert_eq!(scanner.severity_score(&Severity::Critical), 20.0);
}

#[test]
fn test_license_check() {
    let scanner = SecurityScanner::new();

    assert!(matches!(
        scanner.check_license("GPL-3.0"),
        Some(LicenseIssueType::Incompatible)
    ));

    assert!(scanner.check_license("MIT").is_none());
}

#[test]
fn test_security_report() {
    let report = SecurityReport {
        total_vulnerabilities: 5,
        affected_cogs: 3,
        critical_count: 1,
        high_count: 2,
        medium_count: 2,
        low_count: 0,
        max_severity_score: 20.0,
        license_issues: 0,
        supply_chain_risks: 0,
    };

    assert_eq!(report.risk_level(), RiskLevel::Critical);
    assert!(!report.is_clean());
}
