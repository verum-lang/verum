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
// Tests for level module
// Migrated from src/level.rs per CLAUDE.md standards

use verum_verification::level::*;

#[test]
fn test_verification_level_properties() {
    assert!(!VerificationLevel::Runtime.requires_smt());
    assert!(VerificationLevel::Static.requires_smt());
    assert!(VerificationLevel::Proof.requires_smt());

    assert!(VerificationLevel::Runtime.allows_runtime_fallback());
    assert!(VerificationLevel::Static.allows_runtime_fallback());
    assert!(!VerificationLevel::Proof.allows_runtime_fallback());

    assert!(!VerificationLevel::Runtime.generates_proof_certificate());
    assert!(!VerificationLevel::Static.generates_proof_certificate());
    assert!(VerificationLevel::Proof.generates_proof_certificate());
}

#[test]
fn test_verification_level_overhead() {
    assert_eq!(VerificationLevel::Runtime.expected_overhead_ns(), 15);
    assert_eq!(VerificationLevel::Static.expected_overhead_ns(), 0);
    assert_eq!(VerificationLevel::Proof.expected_overhead_ns(), 0);
}

#[test]
fn test_from_annotation() {
    assert_eq!(
        VerificationLevel::from_annotation("runtime"),
        Some(VerificationLevel::Runtime)
    );
    assert_eq!(
        VerificationLevel::from_annotation("static"),
        Some(VerificationLevel::Static)
    );
    assert_eq!(
        VerificationLevel::from_annotation("proof"),
        Some(VerificationLevel::Proof)
    );
    assert_eq!(VerificationLevel::from_annotation("invalid"), None);
}

#[test]
fn test_verification_mode_defaults() {
    let runtime = VerificationMode::runtime();
    assert_eq!(runtime.level, VerificationLevel::Runtime);
    assert!(runtime.config.allow_runtime_fallback);
    assert!(!runtime.config.emit_certificate);

    let proof = VerificationMode::proof();
    assert_eq!(proof.level, VerificationLevel::Proof);
    assert!(!proof.config.allow_runtime_fallback);
    assert!(proof.config.emit_certificate);
}

#[test]
fn test_solver_choice() {
    assert!(!SolverChoice::None.uses_smt());
    assert!(SolverChoice::Auto.uses_smt());
    assert!(SolverChoice::Z3.uses_smt());
    assert!(SolverChoice::CVC5.uses_smt());
}
