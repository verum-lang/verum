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
// Unit tests for refinement_error.rs
//
// Migrated from src/refinement_error.rs to comply with CLAUDE.md test organization.

use verum_common::Text;
use verum_diagnostics::{
    Severity, Span, codes,
    refinement_error::{
        Constraint, CounterExample, RefinementErrorBuilder, SMTTrace, VerificationStep, common,
    },
};

#[test]
fn test_constraint_parsing() {
    let constraint = Constraint::parse("x > 0");
    assert_eq!(constraint.variable, "x");
    assert_eq!(constraint.expression, "x > 0");
}

#[test]
fn test_counterexample() {
    let ce = CounterExample::new()
        .add_assignment("x", "-5")
        .add_assignment("result", "-2.5")
        .with_trace("Found by Z3 solver");

    assert_eq!(ce.assignments.get(&Text::from("x")), Some(&Text::from("-5")));
    assert!(ce.trace.is_some());
}

#[test]
fn test_smt_trace() {
    let trace = SMTTrace::new(false)
        .add_step(VerificationStep::new(1, "assumed", "x >= 0"))
        .add_step(VerificationStep::new(2, "required", "x / 2 >= 0"))
        .with_counterexample(CounterExample::new().add_assignment("x", "-4"));

    assert!(!trace.succeeded);
    assert_eq!(trace.steps.len(), 2);
    assert!(trace.counterexample.is_some());
}

#[test]
fn test_refinement_error_builder() {
    let error = RefinementErrorBuilder::new()
        .constraint("x > 0")
        .actual_value("-5")
        .expected("positive value")
        .span(Span::new("test.vr", 10, 5, 6))
        .suggestion("Use PositiveInt type")
        .build();

    assert_eq!(error.actual_value(), "-5");
    assert_eq!(error.constraint().expression, "x > 0");
    assert_eq!(error.suggestions.len(), 1);
}

#[test]
fn test_positive_constraint_violation() {
    let error = common::positive_constraint_violation("x", "-5", Span::new("main.vr", 3, 12, 13));

    assert_eq!(error.constraint().expression, "x > 0");
    assert_eq!(error.actual_value(), "-5");
    assert!(error.suggestions.len() >= 3);
}

#[test]
fn test_division_by_zero_error() {
    let error = common::division_by_zero("x", "0", Span::new("main.vr", 5, 8, 9));

    assert_eq!(error.constraint().expression, "x != 0");
    assert_eq!(error.actual_value(), "0");
}

#[test]
fn test_to_diagnostic() {
    let error = RefinementErrorBuilder::new()
        .constraint("x > 0")
        .actual_value("-5")
        .span(Span::new("test.vr", 1, 1, 2))
        .suggestion("Fix suggestion")
        .build();

    let diagnostic = error.to_diagnostic();
    assert_eq!(diagnostic.severity(), Severity::Error);
    assert_eq!(diagnostic.code(), Some(codes::E0312));
    assert!(!diagnostic.helps().is_empty());
}
