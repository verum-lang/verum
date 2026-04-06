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
// Unit tests for counterexample.rs
//
// Migrated from src/counterexample.rs to comply with CLAUDE.md test organization.

use verum_smt::counterexample::*;
use verum_common::{List, Map};

#[test]
fn test_counterexample_creation() {
    let mut assignments = Map::new();
    assignments.insert("x".into(), CounterExampleValue::Int(-5));

    let ce = CounterExample::new(assignments, "x > 0".into());

    assert_eq!(ce.assignments.len(), 1);
    assert_eq!(ce.violated_constraint, "x > 0");
    assert_eq!(ce.get("x").unwrap().as_int(), Some(-5));
}

#[test]
fn test_counterexample_display() {
    let mut assignments = Map::new();
    assignments.insert("x".into(), CounterExampleValue::Int(-5));
    assignments.insert("y".into(), CounterExampleValue::Int(10));

    let ce = CounterExample::new(assignments, "x > 0".into());
    let display = format!("{}", ce);

    assert!(display.contains("x = -5"));
    assert!(display.contains("y = 10"));
}

#[test]
fn test_value_display() {
    assert_eq!(format!("{}", CounterExampleValue::Int(42)), "42");
    assert_eq!(format!("{}", CounterExampleValue::Bool(true)), "true");
    assert_eq!(format!("{}", CounterExampleValue::Float(2.5)), "2.5");

    let arr = CounterExampleValue::Array(List::from(vec![
        CounterExampleValue::Int(1),
        CounterExampleValue::Int(2),
        CounterExampleValue::Int(3),
    ]));
    assert_eq!(format!("{}", arr), "[1, 2, 3]");
}

#[test]
fn test_generate_suggestions() {
    let mut assignments = Map::new();
    assignments.insert("x".into(), CounterExampleValue::Int(-5));

    let ce = CounterExample::new(assignments, "x > 0".into());
    let suggestions = generate_suggestions(&ce, "x > 0");

    assert!(!suggestions.is_empty());
    assert!(suggestions.iter().any(|s| s.contains("precondition")));
}

#[test]
fn test_is_minimal() {
    let mut assignments = Map::new();
    assignments.insert("x".into(), CounterExampleValue::Int(5));

    let ce1 = CounterExample::new(assignments.clone(), "test".into());
    assert!(ce1.is_minimal());

    assignments.insert("y".into(), CounterExampleValue::Int(10));
    let ce2 = CounterExample::new(assignments, "test".into());
    assert!(!ce2.is_minimal());
}

#[test]
fn test_value_conversions() {
    let int_val = CounterExampleValue::Int(42);
    assert_eq!(int_val.as_int(), Some(42));
    assert_eq!(int_val.as_float(), Some(42.0));
    assert!(int_val.is_scalar());

    let bool_val = CounterExampleValue::Bool(true);
    assert_eq!(bool_val.as_bool(), Some(true));
    assert!(bool_val.is_scalar());

    let arr = CounterExampleValue::Array(List::from(vec![CounterExampleValue::Int(1)]));
    assert!(!arr.is_scalar());
    assert_eq!(arr.as_array().unwrap().len(), 1);
}
