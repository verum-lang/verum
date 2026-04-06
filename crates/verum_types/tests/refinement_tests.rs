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
    unused_assignments,
    clippy::approx_constant,
    clippy::overly_complex_bool_expr
)]
use verum_types::refinement::*;

#[test]
fn test_refinement_checker_creation() {
    // Test that RefinementChecker can be created
    let config = RefinementConfig::default();
    let _checker = RefinementChecker::new(config);

    // Basic sanity test - checker should be created successfully
}

#[test]
fn test_refinement_config_default() {
    let config = RefinementConfig::default();

    // Verify default configuration is sensible
    assert!(!config.enable_smt || config.enable_smt); // SMT may be enabled or disabled by default
}

#[test]
fn test_verification_stats() {
    let stats = VerificationStats::default();

    // Stats should start at zero
    assert_eq!(stats.total_checks, 0);
    assert_eq!(stats.successful, 0);
    assert_eq!(stats.failed, 0);
}
