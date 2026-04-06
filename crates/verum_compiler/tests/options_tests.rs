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
// Migrated from src/options.rs to comply with CLAUDE.md test organization.

use verum_compiler::options::*;

#[test]
fn test_default_options() {
    let opts = CompilerOptions::default();
    assert_eq!(opts.verify_mode, VerifyMode::Auto);
    assert_eq!(opts.smt_timeout_secs, 30);
    assert_eq!(opts.optimization_level, 0);
    assert!(opts.debug_info);
    assert!(!opts.lto);
}

#[test]
fn test_verify_mode() {
    assert!(VerifyMode::Proof.use_smt());
    assert!(VerifyMode::Auto.use_smt());
    assert!(!VerifyMode::Runtime.use_smt());

    assert!(VerifyMode::Runtime.use_runtime());
    assert!(VerifyMode::Auto.use_runtime());
}

#[test]
fn test_builder() {
    use std::path::PathBuf;

    let opts = CompilerOptions::new(PathBuf::from("test.vr"), PathBuf::from("test"))
        .with_verify_mode(VerifyMode::Proof)
        .with_optimization(3)
        .with_verification_costs(true);

    assert_eq!(opts.verify_mode, VerifyMode::Proof);
    assert_eq!(opts.optimization_level, 3);
    assert!(opts.show_verification_costs);
    // Release mode is determined by optimization_level >= 1
    assert!(opts.optimization_level >= 1);
}
