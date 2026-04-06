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
// Comprehensive tests for advanced SMT features
//
// Tests probe-based strategy selection, unsat core minimization,
// proof extraction, SMT-LIB2 export, and enhanced counterexamples.
//
// REQUIRES API MIGRATION: ComplexityThresholds, TacticKind variants changed
#![allow(unexpected_cfgs)]
#![cfg(feature = "advanced_features_tests_disabled")]

use verum_ast::{
    Expr, ExprKind,
    literal::{IntLit, Literal, LiteralKind},
    span::Span,
};
use verum_smt::proof_term_unified::ProofTerm;
use verum_smt::{
    ComplexityThresholds, CounterExample, CounterExampleCategorizer, CounterExampleValue,
    FailureCategory, ProofExporter, ProofExtractor, ProofMinimizer, SmtCheckMode, SmtLibExporter,
    StrategySelector, TacticKind,
};
use verum_common::{List, Map, Maybe};

// ==================== Test Helpers ====================

fn dummy_expr() -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
        Span::dummy(),
    )
}

fn int_expr(n: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit {
                value: n as i128,
                suffix: None,
            }),
            Span::dummy(),
        )),
        Span::dummy(),
    )
}

// ==================== Strategy Selection Tests ====================

#[test]
fn test_strategy_selector_creation() {
    let selector = StrategySelector::new();
    assert!(selector.enable_auto_selection);
    assert_eq!(selector.fallback_tactic, TacticKind::SMT);
}

#[test]
fn test_strategy_selector_empty_constraints() {
    let selector = StrategySelector::new();
    let empty_constraints: Vec<Expr> = vec![];

    let tactic = selector.select_tactic(&empty_constraints);
    // Should use fallback tactic (SMT)
    let _ = tactic; // Just verify it doesn't panic
}

#[test]
fn test_timeout_estimation_empty() {
    let selector = StrategySelector::new();
    let empty: Vec<Expr> = vec![];

    let timeout = selector.estimate_timeout(&empty);
    assert_eq!(timeout, std::time::Duration::from_secs(1));
}

#[test]
fn test_parallel_strategies_generation() {
    let selector = StrategySelector::new();
    let constraints: Vec<Expr> = vec![];

    let strategies = selector.get_parallel_strategies(&constraints);
    assert!(!strategies.is_empty());
    assert_eq!(strategies[0], TacticKind::SMT);
}

#[test]
fn test_complexity_thresholds() {
    let default = ComplexityThresholds::default();
    assert_eq!(default.small_problem_size, 100.0);
    assert_eq!(default.deep_formula_depth, 20.0);
    assert_eq!(default.many_constants, 50.0);

    let conservative = ComplexityThresholds::conservative();
    assert!(conservative.small_problem_size < default.small_problem_size);

    let aggressive = ComplexityThresholds::aggressive();
    assert!(aggressive.small_problem_size > default.small_problem_size);
}

#[test]
fn test_tactic_kind_descriptions() {
    assert_eq!(TacticKind::SMT.description(), "Default SMT solver");
    assert_eq!(
        TacticKind::Fast.description(),
        "Fast tactic for small problems"
    );
    assert_eq!(
        TacticKind::LinearArithmetic.description(),
        "Linear integer arithmetic"
    );
    assert_eq!(TacticKind::BitVector.description(), "Bit-vector arithmetic");
}

#[test]
fn test_tactic_to_z3() {
    // Just verify all tactics can be converted without panic
    let tactics = [
        TacticKind::SMT,
        TacticKind::Fast,
        TacticKind::Deep,
        TacticKind::LinearArithmetic,
        TacticKind::BitVector,
        TacticKind::NonLinearArithmetic,
        TacticKind::Propositional,
    ];

    for tactic_kind in &tactics {
        let _tactic = tactic_kind.to_tactic();
    }
}

// ==================== Proof Extraction Tests ====================

#[test]
fn test_proof_extractor_creation() {
    let extractor = ProofExtractor::new();
    assert!(extractor.simplify_proofs);
    assert_eq!(extractor.max_depth, 1000);
}

#[test]
fn test_proof_term_axiom() {
    let axiom = ProofTerm::axiom("ax1", dummy_expr());

    assert_eq!(axiom.proof_depth(), 1);
    assert_eq!(axiom.node_count(), 1);

    let axioms = axiom.used_axioms();
    assert_eq!(axioms.len(), 1);
}

#[test]
fn test_proof_term_reflexivity() {
    let refl = ProofTerm::reflexivity(int_expr(42));

    assert_eq!(refl.proof_depth(), 1);
    assert_eq!(refl.node_count(), 1);
}

#[test]
fn test_proof_term_modus_ponens() {
    let premise = ProofTerm::axiom("premise", dummy_expr());
    let implication = ProofTerm::axiom("implication", dummy_expr());

    let mp = ProofTerm::modus_ponens(premise, implication);

    assert_eq!(mp.proof_depth(), 2);
    assert_eq!(mp.node_count(), 3);

    let axioms = mp.used_axioms();
    assert_eq!(axioms.len(), 2);
}

#[test]
fn test_proof_minimization() {
    let refl = ProofTerm::reflexivity(int_expr(42));
    let axiom = ProofTerm::axiom("ax", dummy_expr());

    let trans = ProofTerm::transitivity(refl, axiom.clone());

    let minimized = ProofMinimizer::minimize(&trans);
    // Minimization should simplify the proof
    assert!(minimized.node_count() <= trans.node_count());
}

#[test]
fn test_proof_export_smtlib2() {
    let axiom = ProofTerm::axiom("ax1", dummy_expr());

    let smtlib = ProofExporter::to_smtlib2(&axiom);
    assert!(smtlib.contains("assert"));
    assert!(smtlib.contains("ax1"));
}

#[test]
fn test_proof_export_readable() {
    let axiom = ProofTerm::axiom("test_axiom", dummy_expr());

    let readable = ProofExporter::to_readable(&axiom);
    assert!(readable.contains("Axiom"));
    assert!(readable.contains("test_axiom"));
}

// ==================== SMT-LIB2 Export Tests ====================

#[test]
fn test_smtlib_exporter_creation() {
    let exporter = SmtLibExporter::new();
    // Exporter should be created successfully
    let _ = exporter;
}

#[test]
fn test_smtlib_export_with_logic() {
    let exporter = SmtLibExporter::new().with_logic("QF_LIA");

    let output = exporter.export();
    assert!(output.contains("(set-logic QF_LIA)"));
    assert!(output.contains("(check-sat)"));
    assert!(output.contains("(exit)"));
}

#[test]
fn test_smtlib_export_check_modes() {
    let modes = [
        (SmtCheckMode::CheckSat, "(check-sat)"),
        (SmtCheckMode::GetModel, "(get-model)"),
        (SmtCheckMode::GetUnsatCore, "(get-unsat-core)"),
        (SmtCheckMode::GetProof, "(get-proof)"),
    ];

    for (mode, expected) in &modes {
        let exporter = SmtLibExporter::new().with_check_mode(*mode);
        let output = exporter.export();
        assert!(output.contains(expected));
    }
}

// ==================== Counterexample Tests ====================

#[test]
fn test_counterexample_creation() {
    let mut assignments = Map::new();
    assignments.insert("x".into(), CounterExampleValue::Int(-5));

    let ce = CounterExample::new(assignments, "x > 0".into());

    assert_eq!(ce.violated_constraint, "x > 0");
    assert_eq!(ce.assignments.len(), 1);
    assert!(ce.is_minimal());

    let x_value = ce.get("x").unwrap();
    assert_eq!(x_value.as_int(), Some(-5));
}

#[test]
fn test_counterexample_value_types() {
    let bool_val = CounterExampleValue::Bool(true);
    assert!(bool_val.is_scalar());
    assert_eq!(bool_val.as_bool(), Some(true));

    let int_val = CounterExampleValue::Int(42);
    assert!(int_val.is_scalar());
    assert_eq!(int_val.as_int(), Some(42));

    let float_val = CounterExampleValue::Float(2.5);
    assert!(float_val.is_scalar());
    assert_eq!(float_val.as_float(), Some(2.5));

    let array_val = CounterExampleValue::Array(List::from(vec![
        CounterExampleValue::Int(1),
        CounterExampleValue::Int(2),
    ]));
    assert!(!array_val.is_scalar());
    assert_eq!(array_val.as_array().unwrap().len(), 2);
}

#[test]
fn test_counterexample_categorization() {
    let mut assignments = Map::new();
    assignments.insert("divisor".into(), CounterExampleValue::Int(0));

    let ce = CounterExample::new(assignments, "divisor != 0".into());
    let category = CounterExampleCategorizer::categorize(&ce);

    assert_eq!(category, FailureCategory::DivisionByZero);
}

#[test]
fn test_failure_category_suggestions() {
    let suggestions = CounterExampleCategorizer::suggest_fixes(FailureCategory::DivisionByZero);
    assert!(!suggestions.is_empty());
    assert!(suggestions.iter().any(|s| s.contains("divisor")));

    let neg_suggestions = CounterExampleCategorizer::suggest_fixes(FailureCategory::NegativeValue);
    assert!(
        neg_suggestions
            .iter()
            .any(|s| s.contains("unsigned") || s.contains("Positive"))
    );
}

#[test]
fn test_failure_category_display() {
    assert_eq!(
        format!("{}", FailureCategory::DivisionByZero),
        "Division by Zero"
    );
    assert_eq!(
        format!("{}", FailureCategory::ArithmeticOverflow),
        "Arithmetic Overflow"
    );
    assert_eq!(
        format!("{}", FailureCategory::IndexOutOfBounds),
        "Index Out of Bounds"
    );
}

// ==================== Integration Tests ====================

#[test]
fn test_full_verification_workflow() {
    // This test demonstrates the full workflow with advanced features

    // 1. Create strategy selector
    let selector = StrategySelector::new();

    // 2. Create simple problem (using Expr instead of Z3 AST)
    let constraint = dummy_expr();
    let constraints = vec![constraint];

    // 3. Select strategy
    let _tactic = selector.select_tactic(&constraints);

    // 4. Estimate timeout
    let timeout = selector.estimate_timeout(&constraints);
    assert!(timeout.as_secs() >= 1);

    // 5. Get parallel strategies
    let strategies = selector.get_parallel_strategies(&constraints);
    assert!(!strategies.is_empty());
}

#[test]
fn test_proof_analysis_workflow() {
    // Create a simple proof
    let axiom = ProofTerm::axiom("test", dummy_expr());

    // Analyze it
    let extractor = ProofExtractor::new();
    let analysis = extractor.analyze(&axiom);

    assert_eq!(analysis.depth, 1);
    assert_eq!(analysis.node_count, 1);
    assert!(!analysis.has_quantifiers);
    assert!(analysis.is_simple());

    // Export it
    let smtlib = ProofExporter::to_smtlib2(&axiom);
    assert!(!smtlib.is_empty());
}

// ==================== Performance Smoke Tests ====================

#[test]
fn test_strategy_selection_performance() {
    use std::time::Instant;

    let selector = StrategySelector::new();

    // Create a moderately sized problem
    let constraints: Vec<Expr> = (0..10).map(|i| int_expr(i)).collect();

    let start = Instant::now();
    let _tactic = selector.select_tactic(&constraints);
    let elapsed = start.elapsed();

    // Strategy selection should be fast (< 100ms)
    assert!(elapsed.as_millis() < 100);
}

#[test]
fn test_smtlib_export_performance() {
    use std::time::Instant;

    let mut exporter = SmtLibExporter::new().with_logic("QF_LIA");

    // Add many constraints
    for i in 0..100 {
        // This would normally use actual Expr types
        // For now, just test the exporter can be created
        let _ = i;
    }

    let start = Instant::now();
    let _output = exporter.export();
    let elapsed = start.elapsed();

    // Export should be fast (< 10ms)
    assert!(elapsed.as_millis() < 10);
}
