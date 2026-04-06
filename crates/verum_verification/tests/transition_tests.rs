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
// Tests for transition module
// Migrated from src/transition.rs per CLAUDE.md standards

use verum_common::{List, Text};
use verum_verification::VerificationLevel;
use verum_verification::transition::*;

#[test]
fn test_transition_strategy_thresholds() {
    assert_eq!(
        TransitionStrategy::Conservative.confidence_threshold(),
        0.95
    );
    assert_eq!(TransitionStrategy::Balanced.confidence_threshold(), 0.80);
    assert_eq!(TransitionStrategy::Aggressive.confidence_threshold(), 0.60);
}

#[test]
fn test_code_metrics_stability() {
    let mut metrics = CodeMetrics::default();
    metrics.change_frequency_per_week = 0.05;
    assert!(metrics.is_very_stable());
    assert!(metrics.is_stable());

    metrics.change_frequency_per_week = 0.5;
    assert!(!metrics.is_very_stable());
    assert!(metrics.is_stable());

    metrics.change_frequency_per_week = 2.0;
    assert!(!metrics.is_stable());
}

#[test]
fn test_transition_decision_threshold() {
    let decision = TransitionDecision::transition(
        VerificationLevel::Runtime,
        VerificationLevel::Static,
        0.85,
        15.0,
        10.0,
        Text::from("test"),
        List::new(),
    );

    assert!(decision.passes_threshold(&TransitionStrategy::Balanced));
    assert!(!decision.passes_threshold(&TransitionStrategy::Conservative));
    assert!(decision.passes_threshold(&TransitionStrategy::Aggressive));
}

#[test]
fn test_transition_analyzer() {
    let analyzer = TransitionAnalyzer::new(TransitionStrategy::Balanced);

    let mut metrics = CodeMetrics::default();
    metrics.test_coverage = 0.95;
    metrics.change_frequency_per_week = 0.1;
    metrics.criticality_score = 5;

    let decision = analyzer.analyze_function(
        &Text::from("test_func"),
        VerificationLevel::Runtime,
        &metrics,
    );

    assert!(decision.recommend);
    assert_eq!(decision.to, VerificationLevel::Static);
}
