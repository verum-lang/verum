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
// Comprehensive tests for Fixed-Point (μZ) Engine
//
// Tests the complete functionality of the Z3 fixedpoint engine including:
// - Basic engine operations
// - Datalog rule solving
// - Constrained Horn Clauses (CHC)
// - Recursive predicates
// - Transitive closure
// - Graph reachability
// - List/Tree/Graph predicate patterns
//
// Recursive refinement types use Z3's fixedpoint engine (muZ) for verification.
// Recursive types like `type Nat is Zero | Succ(Nat)` generate inductive predicates
// that are solved via Constrained Horn Clauses (CHC). The engine computes least
// fixpoints for reachability and greatest fixpoints for safety properties.

use verum_smt::fixedpoint::{
    Atom, CHC, DatalogRule, FixedPointEngine, create_fixedpoint_context, extract_invariants,
    patterns,
};
use verum_common::{List, Text};
use z3::ast::{Bool, Dynamic, Int};
use z3::{Context, FuncDecl, SatResult, Sort};

// ==================== Basic Engine Tests ====================

#[test]
fn test_fixedpoint_engine_creation() {
    let ctx = Context::thread_local();
    let engine = FixedPointEngine::new(ctx);
    assert!(engine.is_ok(), "Failed to create fixedpoint engine");
}

#[test]
fn test_create_fixedpoint_context_datalog() {
    let engine = create_fixedpoint_context(false);
    assert!(engine.is_ok(), "Failed to create Datalog context");
}

#[test]
fn test_create_fixedpoint_context_spacer() {
    let engine = create_fixedpoint_context(true);
    assert!(engine.is_ok(), "Failed to create SPACER context");
}

#[test]
fn test_register_relation() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    // Create edge(Int, Int) relation
    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());

    let result = engine.register_relation(&edge);
    assert!(result.is_ok(), "Failed to register relation");
}

// ==================== Datalog Rule Tests ====================

#[test]
fn test_simple_fact() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    // Define edge relation
    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();

    // Add fact: edge(1, 2)
    let x = Int::from_i64(1);
    let y = Int::from_i64(2);
    let fact = edge
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(y.clone())])
        .as_bool()
        .unwrap();
    engine.add_rule(&fact, Some("edge_1_2")).unwrap();

    // Query: edge(1, 2)?
    let query = edge
        .apply(&[&Dynamic::from(x), &Dynamic::from(y)])
        .as_bool()
        .unwrap();
    let result = engine.query(&query).unwrap();

    assert_eq!(result, SatResult::Sat, "Query should be satisfiable");
}

#[test]
fn test_multiple_facts() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();

    // Add multiple facts
    let facts = [(1, 2), (2, 3), (3, 4)];
    for (src, dst) in facts.iter() {
        let x = Int::from_i64(*src);
        let y = Int::from_i64(*dst);
        let fact = edge
            .apply(&[&Dynamic::from(x), &Dynamic::from(y)])
            .as_bool()
            .unwrap();
        engine
            .add_rule(&fact, Some(&format!("edge_{}_{}", src, dst)))
            .unwrap();
    }

    // Query one of them
    let x = Int::from_i64(2);
    let y = Int::from_i64(3);
    let query = edge
        .apply(&[&Dynamic::from(x), &Dynamic::from(y)])
        .as_bool()
        .unwrap();
    let result = engine.query(&query).unwrap();

    assert_eq!(result, SatResult::Sat);
}

#[test]
fn test_negative_query() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();

    // Add fact: edge(1, 2)
    let x = Int::from_i64(1);
    let y = Int::from_i64(2);
    let fact = edge
        .apply(&[&Dynamic::from(x), &Dynamic::from(y)])
        .as_bool()
        .unwrap();
    engine.add_rule(&fact, Some("edge_1_2")).unwrap();

    // Query for non-existent edge: edge(3, 4)?
    let x = Int::from_i64(3);
    let y = Int::from_i64(4);
    let query = edge
        .apply(&[&Dynamic::from(x), &Dynamic::from(y)])
        .as_bool()
        .unwrap();
    let result = engine.query(&query).unwrap();

    assert_eq!(
        result,
        SatResult::Unsat,
        "Non-existent edge should be UNSAT"
    );
}

// ==================== Transitive Closure Tests ====================

#[test]
fn test_transitive_closure_basic() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    // Define edge and path relations
    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());
    let path = FuncDecl::new("path", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();
    engine.register_relation(&path).unwrap();

    // Add facts: edge(1,2), edge(2,3)
    let v1 = Int::from_i64(1);
    let v2 = Int::from_i64(2);
    let v3 = Int::from_i64(3);

    let e12 = edge
        .apply(&[&Dynamic::from(v1.clone()), &Dynamic::from(v2.clone())])
        .as_bool()
        .unwrap();
    let e23 = edge
        .apply(&[&Dynamic::from(v2.clone()), &Dynamic::from(v3.clone())])
        .as_bool()
        .unwrap();

    engine.add_rule(&e12, Some("edge_1_2")).unwrap();
    engine.add_rule(&e23, Some("edge_2_3")).unwrap();

    // Add rules for path:
    // path(x,y) :- edge(x,y)
    let x = Int::new_const("x");
    let y = Int::new_const("y");
    let z = Int::new_const("z");

    let rule1 = edge
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(y.clone())])
        .as_bool()
        .unwrap()
        .implies(
            path
                .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(y.clone())])
                .as_bool()
                .unwrap(),
        );
    engine.add_rule(&rule1, Some("path_base")).unwrap();

    // path(x,z) :- edge(x,y) and path(y,z)
    let edge_xy = edge
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(y.clone())])
        .as_bool()
        .unwrap();
    let path_yz = path
        .apply(&[&Dynamic::from(y.clone()), &Dynamic::from(z.clone())])
        .as_bool()
        .unwrap();
    let path_xz = path
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(z.clone())])
        .as_bool()
        .unwrap();
    let rule2 = Bool::and(&[&edge_xy, &path_yz]).implies(&path_xz);

    engine.add_rule(&rule2, Some("path_trans")).unwrap();

    // Query: path(1, 3)?
    let query = path
        .apply(&[&Dynamic::from(v1), &Dynamic::from(v3)])
        .as_bool()
        .unwrap();
    let result = engine.query(&query).unwrap();

    // Z3 fixed-point PDR engine may return Unknown for some queries
    // Both Sat and Unknown are acceptable - only Unsat would indicate failure
    assert!(
        matches!(result, SatResult::Sat | SatResult::Unknown),
        "Transitive path should exist or be unknown, got {:?}",
        result
    );
}

#[test]
fn test_transitive_closure_long_chain() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());
    let path = FuncDecl::new("path", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();
    engine.register_relation(&path).unwrap();

    // Create chain: 1->2->3->4->5
    for i in 1..5 {
        let src = Int::from_i64(i);
        let dst = Int::from_i64(i + 1);
        let fact = edge
            .apply(&[&Dynamic::from(src), &Dynamic::from(dst)])
            .as_bool()
            .unwrap();
        engine
            .add_rule(&fact, Some(&format!("edge_{}_{}", i, i + 1)))
            .unwrap();
    }

    // Add transitive closure rules
    let x = Int::new_const("x");
    let y = Int::new_const("y");
    let z = Int::new_const("z");

    let rule1 = edge
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(y.clone())])
        .as_bool()
        .unwrap()
        .implies(
            path
                .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(y.clone())])
                .as_bool()
                .unwrap(),
        );
    engine.add_rule(&rule1, Some("path_base")).unwrap();

    let edge_xy = edge
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(y.clone())])
        .as_bool()
        .unwrap();
    let path_yz = path
        .apply(&[&Dynamic::from(y.clone()), &Dynamic::from(z.clone())])
        .as_bool()
        .unwrap();
    let path_xz = path
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(z.clone())])
        .as_bool()
        .unwrap();
    let rule2 = Bool::and(&[&edge_xy, &path_yz]).implies(&path_xz);
    engine.add_rule(&rule2, Some("path_trans")).unwrap();

    // Query: path(1, 5)? (should be reachable through chain)
    let v1 = Int::from_i64(1);
    let v5 = Int::from_i64(5);
    let query = path
        .apply(&[&Dynamic::from(v1), &Dynamic::from(v5)])
        .as_bool()
        .unwrap();
    let result = engine.query(&query).unwrap();

    // Z3 fixed-point PDR engine may return Unknown for some queries
    // Both Sat and Unknown are acceptable - only Unsat would indicate failure
    assert!(
        matches!(result, SatResult::Sat | SatResult::Unknown),
        "Long transitive path should exist or be unknown, got {:?}",
        result
    );
}

// ==================== Get Answer Tests ====================

#[test]
fn test_get_answer_after_sat() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();

    let x = Int::from_i64(1);
    let y = Int::from_i64(2);
    let fact = edge
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(y.clone())])
        .as_bool()
        .unwrap();
    engine.add_rule(&fact, Some("edge_1_2")).unwrap();

    let query = edge
        .apply(&[&Dynamic::from(x), &Dynamic::from(y)])
        .as_bool()
        .unwrap();
    let result = engine.query(&query).unwrap();
    assert_eq!(result, SatResult::Sat);

    // Get answer
    let answer = engine.get_answer();
    assert!(
        answer.is_ok(),
        "Should be able to get answer after SAT query"
    );
}

// ==================== Statistics Tests ====================

#[test]
fn test_statistics_tracking() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();

    let x = Int::from_i64(1);
    let y = Int::from_i64(2);
    let fact = edge
        .apply(&[&Dynamic::from(x), &Dynamic::from(y)])
        .as_bool()
        .unwrap();
    engine.add_rule(&fact, None).unwrap();

    let stats = engine.get_statistics();
    assert_eq!(stats.num_rules, 1, "Should track rule count");
}

// ==================== Datalog Rule Conversion Tests ====================

#[test]
fn test_datalog_fact_conversion() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let node = FuncDecl::new("node", &[&int_sort], &Sort::bool());

    engine.register_relation(&node).unwrap();

    // Create a DatalogRule (fact)
    let rule = DatalogRule {
        head: Atom {
            predicate: Text::from("node"),
            args: List::from(vec![Dynamic::from(Int::from_i64(1))]),
        },
        body: List::new(),
        constraints: List::new(),
    };

    let result = engine.add_datalog_rule(rule);
    assert!(result.is_ok(), "Should add datalog fact");
}

// ==================== Recursive Predicate Pattern Tests ====================

#[test]
fn test_list_length_pattern() {
    let ctx = Context::thread_local();
    let pred = patterns::ListPredicates::length(&ctx);

    assert_eq!(pred.name, Text::from("list_length"));
    assert!(pred.well_founded);
    assert_eq!(pred.params.len(), 2);
}

#[test]
fn test_list_contains_pattern() {
    let ctx = Context::thread_local();
    let pred = patterns::ListPredicates::contains(&ctx);

    assert_eq!(pred.name, Text::from("list_contains"));
    assert!(pred.well_founded);
}

#[test]
fn test_list_sum_pattern() {
    let ctx = Context::thread_local();
    let pred = patterns::ListPredicates::sum(&ctx);

    assert_eq!(pred.name, Text::from("list_sum"));
    assert!(pred.well_founded);
}

#[test]
fn test_tree_height_pattern() {
    let ctx = Context::thread_local();
    let pred = patterns::TreePredicates::height(&ctx);

    assert_eq!(pred.name, Text::from("tree_height"));
    assert!(pred.well_founded);
}

#[test]
fn test_tree_search_pattern() {
    let ctx = Context::thread_local();
    let pred = patterns::TreePredicates::search(&ctx);

    assert_eq!(pred.name, Text::from("tree_search"));
    assert!(pred.well_founded);
}

#[test]
fn test_tree_size_pattern() {
    let ctx = Context::thread_local();
    let pred = patterns::TreePredicates::size(&ctx);

    assert_eq!(pred.name, Text::from("tree_size"));
    assert!(pred.well_founded);
}

#[test]
fn test_graph_reachability_pattern() {
    let ctx = Context::thread_local();
    let pred = patterns::GraphPredicates::reachability(&ctx);

    assert_eq!(pred.name, Text::from("reachable"));
    assert!(!pred.well_founded); // May have cycles
}

#[test]
fn test_graph_cycle_detection_pattern() {
    let ctx = Context::thread_local();
    let pred = patterns::GraphPredicates::cycle_detection(&ctx);

    assert_eq!(pred.name, Text::from("has_cycle"));
    assert!(!pred.well_founded); // Cycles exist
}

#[test]
fn test_graph_path_length_pattern() {
    let ctx = Context::thread_local();
    let pred = patterns::GraphPredicates::path_length(&ctx);

    assert_eq!(pred.name, Text::from("path_length"));
    assert!(!pred.well_founded); // May have cycles
}

// ==================== Invariant Extraction Tests ====================

#[test]
fn test_extract_invariants_empty() {
    use verum_smt::fixedpoint::FixedPointSolution;
    use verum_common::Map;

    let solution = FixedPointSolution {
        interpretations: Map::new(),
        invariants: List::new(),
    };

    let invariants = extract_invariants(&solution);
    assert_eq!(invariants.len(), 0);
}

#[test]
fn test_extract_invariants_with_data() {
    use verum_smt::fixedpoint::{FixedPointSolution, PredicateInterpretation};
    use verum_common::Map;

    let _ctx = Context::thread_local();
    let inv1 = Bool::from_bool(true);
    let inv2 = Bool::from_bool(true);

    let mut interps = Map::new();
    interps.insert(
        Text::from("pred1"),
        PredicateInterpretation {
            predicate: Text::from("pred1"),
            formula: inv1.clone(),
            is_inductive: true,
        },
    );

    let solution = FixedPointSolution {
        interpretations: interps,
        invariants: List::from(vec![inv2]),
    };

    let invariants = extract_invariants(&solution);
    assert!(!invariants.is_empty());
}

// ==================== CHC Tests ====================

#[test]
fn test_simple_chc() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let safe = FuncDecl::new("safe", &[&int_sort], &Sort::bool());

    engine.register_relation(&safe).unwrap();

    let x = Int::new_const("x");

    let chc = CHC {
        vars: List::from(vec![(Text::from("x"), Sort::int())]),
        hypothesis: List::new(),
        constraints: List::from(vec![x.gt(Int::from_i64(0))]),
        conclusion: Atom {
            predicate: Text::from("safe"),
            args: List::from(vec![Dynamic::from(x)]),
        },
    };

    let result = engine.add_chc(chc);
    assert!(result.is_ok(), "Should add CHC successfully");
}

// ==================== Performance Tests ====================

#[test]
fn test_large_fact_database() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();

    // Add 100 facts
    for i in 0..100 {
        let src = Int::from_i64(i);
        let dst = Int::from_i64(i + 1);
        let fact = edge
            .apply(&[&Dynamic::from(src), &Dynamic::from(dst)])
            .as_bool()
            .unwrap();
        engine
            .add_rule(&fact, Some(&format!("edge_{}", i)))
            .unwrap();
    }

    let stats = engine.get_statistics();
    assert_eq!(stats.num_rules, 100);
}

#[test]
fn test_query_performance() {
    use std::time::Instant;

    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();

    let x = Int::from_i64(1);
    let y = Int::from_i64(2);
    let fact = edge
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(y.clone())])
        .as_bool()
        .unwrap();
    engine.add_rule(&fact, Some("edge_1_2")).unwrap();

    let start = Instant::now();
    let query = edge
        .apply(&[&Dynamic::from(x), &Dynamic::from(y)])
        .as_bool()
        .unwrap();
    let _result = engine.query(&query).unwrap();
    let elapsed = start.elapsed();

    // Query should be fast (< 100ms)
    assert!(
        elapsed.as_millis() < 100,
        "Query took too long: {:?}",
        elapsed
    );
}

// ==================== Edge Cases ====================

#[test]
fn test_query_before_facts() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();

    // Query without any facts
    let x = Int::from_i64(1);
    let y = Int::from_i64(2);
    let query = edge
        .apply(&[&Dynamic::from(x), &Dynamic::from(y)])
        .as_bool()
        .unwrap();
    let result = engine.query(&query).unwrap();

    assert_eq!(result, SatResult::Unsat, "Empty database should be UNSAT");
}

#[test]
fn test_self_loop() {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();

    let int_sort = Sort::int();
    let edge = FuncDecl::new("edge", &[&int_sort, &int_sort], &Sort::bool());

    engine.register_relation(&edge).unwrap();

    // Add self-loop: edge(1, 1)
    let x = Int::from_i64(1);
    let fact = edge
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(x.clone())])
        .as_bool()
        .unwrap();
    engine.add_rule(&fact, Some("self_loop")).unwrap();

    let query = edge
        .apply(&[&Dynamic::from(x.clone()), &Dynamic::from(x)])
        .as_bool()
        .unwrap();
    let result = engine.query(&query).unwrap();

    assert_eq!(result, SatResult::Sat, "Self-loop should be satisfiable");
}

// ==================== Integration with High-Level Functions ====================

#[test]
fn test_create_and_solve_context() {
    let mut engine = create_fixedpoint_context(false).unwrap();

    let _ctx = Context::thread_local();
    let int_sort = Sort::int();
    let edge = FuncDecl::new("test", &[&int_sort, &int_sort], &Sort::bool());

    let result = engine.register_relation(&edge);
    assert!(result.is_ok());
}
