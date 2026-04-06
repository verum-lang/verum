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
// Tests for static_verification module
// Migrated from src/static_verification.rs per CLAUDE.md standards
// FIXED (Session 23): Tests enabled

// #![cfg(feature = "static_verification_tests_disabled")]

use verum_smt::static_verification::*;
use verum_common::Text;

#[test]
fn test_bounds_check_verification_proved() {
    let verifier = StaticVerifier::default_config();

    // Create a bounds check constraint that should be provable
    // with the right preconditions
    let constraint = SafetyConstraint {
        id: Text::from("bounds_1"),
        formula: ConstraintFormula::BoundsCheck {
            index_var: Text::from("i"),
            length_var: Text::from("len"),
            length_value: Some(10),
        },
        source_location: None,
        category: ConstraintCategory::BoundsCheck,
        variables: vec![VariableInfo {
            name: Text::from("i"),
            var_type: VariableType::Int,
            source_name: None,
        }]
        .into(),
        description: Text::from("Array bounds check: 0 <= i < 10"),
    };

    // Add precondition that i is in range
    let mut verifier = verifier;
    verifier.add_assumption(SafetyConstraint {
        id: Text::from("pre_i"),
        formula: ConstraintFormula::BoundsCheck {
            index_var: Text::from("i"),
            length_var: Text::from("len"),
            length_value: Some(10),
        },
        source_location: None,
        category: ConstraintCategory::BoundsCheck,
        variables: vec![].into(),
        description: Text::from("Precondition: 0 <= i < 10"),
    });

    let result = verifier.verify(&constraint);

    // With precondition, should be proved
    assert!(result.can_eliminate_check());
}

#[test]
fn test_division_safe_unprovable() {
    let verifier = StaticVerifier::default_config();

    // Division safety without precondition - should find counterexample
    let constraint = SafetyConstraint {
        id: Text::from("div_1"),
        formula: ConstraintFormula::DivisionSafe {
            divisor_var: Text::from("d"),
        },
        source_location: None,
        category: ConstraintCategory::Arithmetic,
        variables: vec![VariableInfo {
            name: Text::from("d"),
            var_type: VariableType::Int,
            source_name: None,
        }]
        .into(),
        description: Text::from("Division by zero check: d != 0"),
    };

    let result = verifier.verify(&constraint);

    // Without precondition, should find counterexample (d = 0)
    assert!(result.needs_runtime_check());
}

#[test]
fn test_cbgr_batch_analyzer() {
    let analyzer = CbgrBatchAnalyzer::new(StaticVerificationConfig::default());
    let mut analyzer = analyzer;

    // Add multiple constraints
    let c1 = SafetyConstraint {
        id: Text::from("cbgr_1"),
        formula: ConstraintFormula::NonNull {
            ptr_name: Text::from("p"),
        },
        source_location: None,
        category: ConstraintCategory::NullCheck,
        variables: vec![].into(),
        description: Text::from("Null check"),
    };

    analyzer.analyze_check(c1);

    let stats = analyzer.elimination_stats();
    assert_eq!(stats.total_checks, 1);
}

#[test]
fn test_verification_config() {
    let config = StaticVerificationConfig::default();
    assert_eq!(config.constraint_timeout_ms, 100);
    assert!(config.enable_proofs);
    assert!(config.enable_unsat_cores);
}
