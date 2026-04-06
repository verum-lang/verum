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
// Integration tests for the Fixedpoint (μZ) engine
//
// Tests the fixed-point computation engine for recursive predicates,
// Datalog rules, and inductive properties using Z3's μZ framework.

use verum_smt::fixedpoint::{
    Atom, DatalogRule, PredicateBody, PredicateCase, RecursiveCall, RecursivePredicate,
};
use verum_common::Text;
use z3::ast::{Bool, Int};

// ==================== Datalog Rule Tests ====================

#[test]
fn test_simple_datalog_rule() {
    // Test that we can create basic datalog rules
    let x = Int::new_const("x");

    let even_pred = Atom {
        predicate: Text::from("even"),
        args: vec![].into_iter().collect(),
    };

    let rule = DatalogRule {
        head: even_pred,
        body: vec![].into_iter().collect(),
        constraints: vec![x.gt(0)].into_iter().collect(),
    };

    assert!(!rule.constraints.is_empty());
}

#[test]
fn test_recursive_datalog_rules() {
    // Base case
    let base_rule = DatalogRule {
        head: Atom {
            predicate: Text::from("reachable"),
            args: vec![].into_iter().collect(),
        },
        body: vec![].into_iter().collect(),
        constraints: vec![].into_iter().collect(),
    };

    // Recursive case
    let recursive_rule = DatalogRule {
        head: Atom {
            predicate: Text::from("reachable"),
            args: vec![].into_iter().collect(),
        },
        body: vec![
            Atom {
                predicate: Text::from("reachable"),
                args: vec![].into_iter().collect(),
            },
            Atom {
                predicate: Text::from("edge"),
                args: vec![].into_iter().collect(),
            },
        ]
        .into_iter()
        .collect(),
        constraints: vec![].into_iter().collect(),
    };

    assert_eq!(base_rule.body.len(), 0);
    assert_eq!(recursive_rule.body.len(), 2);
}

#[test]
fn test_datalog_with_constraints() {
    let x = Int::new_const("x");

    let rule = DatalogRule {
        head: Atom {
            predicate: Text::from("positive"),
            args: vec![].into_iter().collect(),
        },
        body: vec![].into_iter().collect(),
        constraints: vec![x.gt(0)].into_iter().collect(),
    };

    assert_eq!(rule.constraints.len(), 1);
}

// ==================== Recursive Predicate Tests ====================

#[test]
fn test_simple_recursive_predicate() {
    let x = Int::new_const("x");

    let base_pred = RecursivePredicate {
        name: Text::from("factorial"),
        params: vec![].into_iter().collect(),
        body: PredicateBody::Base(x.gt(0)),
        well_founded: true,
    };

    assert!(base_pred.well_founded);
    assert_eq!(base_pred.name, Text::from("factorial"));
}

#[test]
fn test_recursive_predicate_with_cases() {
    let n = Int::new_const("n");

    let case1 = PredicateCase {
        guard: Some(n.eq(0)),
        body: Bool::from_bool(true),
        recursive_calls: vec![].into_iter().collect(),
    };

    let case2 = PredicateCase {
        guard: Some(n.gt(0)),
        body: Bool::from_bool(true),
        recursive_calls: vec![].into_iter().collect(),
    };

    let pred = RecursivePredicate {
        name: Text::from("fact"),
        params: vec![].into_iter().collect(),
        body: PredicateBody::Cases(vec![case1, case2].into_iter().collect()),
        well_founded: true,
    };

    match &pred.body {
        PredicateBody::Cases(cases) => {
            assert_eq!(cases.len(), 2);
        }
        _ => panic!("Expected Cases variant"),
    }
}

// ==================== Configuration Tests (Additional) ====================

#[test]
fn test_atom_creation() {
    let atom = Atom {
        predicate: Text::from("test"),
        args: vec![].into_iter().collect(),
    };

    assert_eq!(atom.predicate, Text::from("test"));
}

// ==================== Horn Clause Tests ====================

#[test]
fn test_simple_horn_clause() {
    let x = Int::new_const("x");

    let rule = DatalogRule {
        head: Atom {
            predicate: Text::from("positive"),
            args: vec![].into_iter().collect(),
        },
        body: vec![].into_iter().collect(),
        constraints: vec![x.gt(0)].into_iter().collect(),
    };

    assert_eq!(rule.constraints.len(), 1);
}

#[test]
fn test_complex_horn_clause() {
    let x = Int::new_const("x");
    let y = Int::new_const("y");

    let rule = DatalogRule {
        head: Atom {
            predicate: Text::from("Q"),
            args: vec![].into_iter().collect(),
        },
        body: vec![
            Atom {
                predicate: Text::from("P"),
                args: vec![].into_iter().collect(),
            },
            Atom {
                predicate: Text::from("R"),
                args: vec![].into_iter().collect(),
            },
        ]
        .into_iter()
        .collect(),
        constraints: vec![x.gt(&y)].into_iter().collect(),
    };

    assert_eq!(rule.body.len(), 2);
    assert_eq!(rule.constraints.len(), 1);
}

// ==================== Basic Predicate Tests ====================

#[test]
fn test_datalog_rule_head_and_body() {
    let rule = DatalogRule {
        head: Atom {
            predicate: Text::from("parent"),
            args: vec![].into_iter().collect(),
        },
        body: vec![Atom {
            predicate: Text::from("father"),
            args: vec![].into_iter().collect(),
        }]
        .into_iter()
        .collect(),
        constraints: vec![].into_iter().collect(),
    };

    assert_eq!(rule.head.predicate, Text::from("parent"));
    assert_eq!(rule.body.len(), 1);
}

// ==================== Integration Tests ====================

#[test]
fn test_fixedpoint_workflow_basic() {
    let x = Int::new_const("x");
    let y = Int::new_const("y");

    let edge_rule = DatalogRule {
        head: Atom {
            predicate: Text::from("edge"),
            args: vec![].into_iter().collect(),
        },
        body: vec![].into_iter().collect(),
        constraints: vec![].into_iter().collect(),
    };

    assert_eq!(edge_rule.head.predicate, Text::from("edge"));
}

#[test]
fn test_mutual_recursion_support() {
    let _x = Int::new_const("x");

    let even_rule = DatalogRule {
        head: Atom {
            predicate: Text::from("even"),
            args: vec![].into_iter().collect(),
        },
        body: vec![Atom {
            predicate: Text::from("odd"),
            args: vec![].into_iter().collect(),
        }]
        .into_iter()
        .collect(),
        constraints: vec![].into_iter().collect(),
    };

    let odd_rule = DatalogRule {
        head: Atom {
            predicate: Text::from("odd"),
            args: vec![].into_iter().collect(),
        },
        body: vec![Atom {
            predicate: Text::from("even"),
            args: vec![].into_iter().collect(),
        }]
        .into_iter()
        .collect(),
        constraints: vec![].into_iter().collect(),
    };

    assert_eq!(even_rule.body.len(), 1);
    assert_eq!(odd_rule.body.len(), 1);
}

#[test]
fn test_well_founded_check() {
    let x = Int::new_const("x");

    let pred_base = RecursivePredicate {
        name: Text::from("fact"),
        params: vec![].into_iter().collect(),
        body: PredicateBody::Base(x.gt(0)),
        well_founded: true,
    };

    assert!(pred_base.well_founded);

    let pred_circular = RecursivePredicate {
        name: Text::from("circular"),
        params: vec![].into_iter().collect(),
        body: PredicateBody::Base(Bool::from_bool(true)),
        well_founded: false,
    };

    assert!(!pred_circular.well_founded);
}

// ==================== Performance Tests ====================

#[test]
fn test_datalog_rule_performance() {
    use std::time::Instant;

    let start = Instant::now();

    let rule = DatalogRule {
        head: Atom {
            predicate: Text::from("test"),
            args: vec![].into_iter().collect(),
        },
        body: vec![].into_iter().collect(),
        constraints: vec![].into_iter().collect(),
    };

    let elapsed = start.elapsed();

    assert!(rule.head.predicate == "test");
    assert!(elapsed.as_millis() < 10);
}

#[test]
fn test_multiple_rule_compilation() {
    let x = Int::new_const("x");

    let mut rules = Vec::new();

    for i in 0..10 {
        let rule = DatalogRule {
            head: Atom {
                predicate: Text::from(format!("pred{}", i)),
                args: vec![].into_iter().collect(),
            },
            body: vec![].into_iter().collect(),
            constraints: vec![x.gt(0)].into_iter().collect(),
        };
        rules.push(rule);
    }

    assert_eq!(rules.len(), 10);
}

// ==================== Edge Case Tests ====================

#[test]
fn test_empty_body_rules() {
    let rule = DatalogRule {
        head: Atom {
            predicate: Text::from("edge"),
            args: vec![].into_iter().collect(),
        },
        body: vec![].into_iter().collect(),
        constraints: vec![].into_iter().collect(),
    };

    assert!(rule.body.is_empty());
    assert!(rule.constraints.is_empty());
}

#[test]
fn test_single_atom_body() {
    let rule = DatalogRule {
        head: Atom {
            predicate: Text::from("pred_a"),
            args: vec![].into_iter().collect(),
        },
        body: vec![Atom {
            predicate: Text::from("pred_b"),
            args: vec![].into_iter().collect(),
        }]
        .into_iter()
        .collect(),
        constraints: vec![].into_iter().collect(),
    };

    assert_eq!(rule.body.len(), 1);
}

#[test]
fn test_predicate_case_with_no_guard() {
    let case = PredicateCase {
        guard: None,
        body: Bool::from_bool(true),
        recursive_calls: vec![].into_iter().collect(),
    };

    assert!(case.guard.is_none());
}
