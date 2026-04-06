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
//! Comprehensive tests for proof generation and validation
//!
//! Tests the complete proof generation pipeline:
//! - Z3 proof extraction
//! - Proof validation
//! - Proof minimization
//! - Integration with verification workflow
//!
//! Proof terms are first-class evidence values following the Curry-Howard correspondence.
//! A `Proof<P>` is evidence of proposition P. Construction via modus ponens, proof by cases
//! (or_elim), lambda abstraction (introduce hypothesis), and SMT discharge. Proofs are
//! type-erased at runtime (proof irrelevance for the Prop universe).

// Use the proof_extraction module's ProofTerm (with Text formulas)
use verum_smt::proof_extraction::ProofTerm;
use verum_smt::{ProofExtractor, ProofFormatter, ProofMinimizer, ProofValidation};
use verum_common::{List, Text};

// ==================== Proof Extraction Tests ====================

#[test]
fn test_proof_extractor_creation() {
    let extractor = ProofExtractor::new();
    assert!(extractor.simplify_proofs);
    assert_eq!(extractor.max_depth, 1000);
}

#[test]
fn test_proof_validation_simple_axiom() {
    let extractor = ProofExtractor::new();

    let axiom = ProofTerm::Axiom {
        name: Text::from("test_axiom"),
        formula: Text::from("x > 0"),
    };

    let validation = extractor.validate_proof(&axiom);

    assert!(validation.is_valid);
    assert!(validation.is_ok());
    assert_eq!(validation.errors.len(), 0);
    assert!(validation.summary().contains("valid"));
}

#[test]
fn test_proof_validation_empty_formula() {
    let extractor = ProofExtractor::new();

    let axiom = ProofTerm::Axiom {
        name: Text::from("test"),
        formula: Text::from(""),
    };

    let validation = extractor.validate_proof(&axiom);

    assert!(!validation.is_valid);
    assert!(!validation.is_ok());
    assert!(!validation.errors.is_empty());
    assert!(validation.errors[0].contains("empty formula"));
}

#[test]
fn test_proof_validation_modus_ponens() {
    let extractor = ProofExtractor::new();

    let premise = ProofTerm::Axiom {
        name: Text::from("premise"),
        formula: Text::from("A"),
    };

    let implication = ProofTerm::Axiom {
        name: Text::from("impl"),
        formula: Text::from("A => B"),
    };

    let mp = ProofTerm::ModusPonens {
        premise: Box::new(premise),
        implication: Box::new(implication),
    };

    let validation = extractor.validate_proof(&mp);

    assert!(validation.is_valid);
    assert_eq!(mp.proof_depth(), 2);
    assert_eq!(mp.node_count(), 3);
}

#[test]
fn test_proof_validation_symmetry_valid() {
    let extractor = ProofExtractor::new();

    let equality = ProofTerm::Axiom {
        name: Text::from("eq"),
        formula: Text::from("x = y"),
    };

    let sym = ProofTerm::Symmetry {
        equality: Box::new(equality),
    };

    let validation = extractor.validate_proof(&sym);

    assert!(validation.is_valid);
}

#[test]
fn test_proof_validation_symmetry_invalid() {
    let extractor = ProofExtractor::new();

    // Symmetry should only apply to equalities
    let non_equality = ProofTerm::Axiom {
        name: Text::from("ne"),
        formula: Text::from("x > y"),
    };

    let sym = ProofTerm::Symmetry {
        equality: Box::new(non_equality),
    };

    let validation = extractor.validate_proof(&sym);

    assert!(!validation.is_valid);
    assert!(!validation.errors.is_empty());
}

#[test]
fn test_proof_validation_transitivity() {
    let extractor = ProofExtractor::new();

    let left = ProofTerm::Axiom {
        name: Text::from("left"),
        formula: Text::from("x = y"),
    };

    let right = ProofTerm::Axiom {
        name: Text::from("right"),
        formula: Text::from("y = z"),
    };

    let trans = ProofTerm::Transitivity {
        left: Box::new(left),
        right: Box::new(right),
    };

    let validation = extractor.validate_proof(&trans);

    assert!(validation.is_valid);
}

// ==================== Proof Minimization Tests ====================

#[test]
fn test_proof_minimization_removes_reflexivity() {
    let refl = ProofTerm::Reflexivity {
        term: Text::from("x"),
    };

    let axiom = ProofTerm::Axiom {
        name: Text::from("ax"),
        formula: Text::from("x = y"),
    };

    let trans = ProofTerm::Transitivity {
        left: Box::new(refl),
        right: Box::new(axiom.clone()),
    };

    let minimized = ProofMinimizer::minimize(&trans);

    // Should simplify to just the axiom
    assert_eq!(minimized, axiom);
}

#[test]
fn test_proof_minimization_nested() {
    let refl1 = ProofTerm::Reflexivity {
        term: Text::from("a"),
    };

    let refl2 = ProofTerm::Reflexivity {
        term: Text::from("b"),
    };

    let inner_trans = ProofTerm::Transitivity {
        left: Box::new(refl1),
        right: Box::new(refl2.clone()),
    };

    let outer_trans = ProofTerm::Transitivity {
        left: Box::new(inner_trans),
        right: Box::new(refl2.clone()),
    };

    let minimized = ProofMinimizer::minimize(&outer_trans);

    // Should collapse to single reflexivity
    assert_eq!(minimized, refl2);
}

#[test]
fn test_minimize_proof_method() {
    let extractor = ProofExtractor::new();

    let refl = ProofTerm::Reflexivity {
        term: Text::from("x"),
    };

    let axiom = ProofTerm::Axiom {
        name: Text::from("ax"),
        formula: Text::from("x = y"),
    };

    let trans = ProofTerm::Transitivity {
        left: Box::new(refl),
        right: Box::new(axiom.clone()),
    };

    let minimized = extractor.minimize_proof(&trans);
    assert_eq!(minimized, axiom);
}

// ==================== Proof Formatting Tests ====================

#[test]
fn test_proof_formatter_axiom() {
    let formatter = ProofFormatter;

    let axiom = ProofTerm::Axiom {
        name: Text::from("test_axiom"),
        formula: Text::from("x > 0"),
    };

    let formatted = formatter.format(&axiom);

    assert!(formatted.contains("Axiom"));
    assert!(formatted.contains("test_axiom"));
    assert!(formatted.contains("x > 0"));
}

#[test]
fn test_proof_formatter_modus_ponens() {
    let formatter = ProofFormatter;

    let premise = ProofTerm::Axiom {
        name: Text::from("p"),
        formula: Text::from("A"),
    };

    let implication = ProofTerm::Axiom {
        name: Text::from("i"),
        formula: Text::from("A => B"),
    };

    let mp = ProofTerm::ModusPonens {
        premise: Box::new(premise),
        implication: Box::new(implication),
    };

    let formatted = formatter.format(&mp);

    assert!(formatted.contains("Modus Ponens"));
    assert!(formatted.contains("A"));
    assert!(formatted.contains("A => B"));
}

#[test]
fn test_proof_formatter_theory_lemma() {
    let formatter = ProofFormatter;

    let lemma = ProofTerm::TheoryLemma {
        theory: Text::from("arithmetic"),
        lemma: Text::from("x + 0 = x"),
    };

    let formatted = formatter.format(&lemma);

    assert!(formatted.contains("Theory Lemma"));
    assert!(formatted.contains("arithmetic"));
    assert!(formatted.contains("x + 0 = x"));
}

#[test]
fn test_proof_formatter_reflexivity() {
    let formatter = ProofFormatter;

    let refl = ProofTerm::Reflexivity {
        term: Text::from("x"),
    };

    let formatted = formatter.format(&refl);

    assert!(formatted.contains("Reflexivity"));
    assert!(formatted.contains("x = x"));
}

// ==================== Proof Analysis Tests ====================

#[test]
fn test_proof_term_conclusion() {
    let axiom = ProofTerm::Axiom {
        name: Text::from("test"),
        formula: Text::from("x > 0"),
    };

    let conclusion = axiom.conclusion();
    assert_eq!(conclusion, Text::from("x > 0"));
}

#[test]
fn test_proof_term_used_axioms() {
    let axiom = ProofTerm::Axiom {
        name: Text::from("ax1"),
        formula: Text::from("x > 0"),
    };

    let axioms = axiom.used_axioms();
    assert_eq!(axioms.len(), 1);
    assert!(axioms.contains(&Text::from("ax1")));
}

#[test]
fn test_proof_term_used_axioms_nested() {
    let axiom1 = ProofTerm::Axiom {
        name: Text::from("ax1"),
        formula: Text::from("A"),
    };

    let axiom2 = ProofTerm::Axiom {
        name: Text::from("ax2"),
        formula: Text::from("A => B"),
    };

    let mp = ProofTerm::ModusPonens {
        premise: Box::new(axiom1),
        implication: Box::new(axiom2),
    };

    let axioms = mp.used_axioms();
    assert_eq!(axioms.len(), 2);
    assert!(axioms.contains(&Text::from("ax1")));
    assert!(axioms.contains(&Text::from("ax2")));
}

#[test]
fn test_proof_depth() {
    let axiom = ProofTerm::Axiom {
        name: Text::from("test"),
        formula: Text::from("x > 0"),
    };

    assert_eq!(axiom.proof_depth(), 1);

    let refl = ProofTerm::Reflexivity {
        term: Text::from("x"),
    };

    let trans = ProofTerm::Transitivity {
        left: Box::new(axiom.clone()),
        right: Box::new(refl),
    };

    assert_eq!(trans.proof_depth(), 2);
}

#[test]
fn test_proof_node_count() {
    let axiom = ProofTerm::Axiom {
        name: Text::from("test"),
        formula: Text::from("x > 0"),
    };

    assert_eq!(axiom.node_count(), 1);

    let refl = ProofTerm::Reflexivity {
        term: Text::from("x"),
    };

    let trans = ProofTerm::Transitivity {
        left: Box::new(axiom),
        right: Box::new(refl),
    };

    assert_eq!(trans.node_count(), 3); // 1 + 1 + 1 (trans node + left + right)
}

// ==================== Proof Validation Result Tests ====================

#[test]
fn test_proof_validation_summary_valid() {
    let validation = ProofValidation {
        is_valid: true,
        errors: List::new(),
        warnings: List::new(),
    };

    let summary = validation.summary();
    assert!(summary.contains("valid"));
}

#[test]
fn test_proof_validation_summary_with_warnings() {
    let validation = ProofValidation {
        is_valid: true,
        errors: List::new(),
        warnings: {
            let mut w = List::new();
            w.push(Text::from("Warning 1"));
            w.push(Text::from("Warning 2"));
            w
        },
    };

    let summary = validation.summary();
    assert!(summary.contains("valid"));
    assert!(summary.contains("2"));
    assert!(summary.contains("warnings"));
}

#[test]
fn test_proof_validation_summary_invalid() {
    let validation = ProofValidation {
        is_valid: false,
        errors: {
            let mut e = List::new();
            e.push(Text::from("Error 1"));
            e
        },
        warnings: List::new(),
    };

    let summary = validation.summary();
    assert!(summary.contains("invalid"));
    assert!(summary.contains("1"));
    assert!(summary.contains("errors"));
}

// ==================== Proof Extractor Integration Tests ====================

#[test]
fn test_extract_and_minimize_integration() {
    let extractor = ProofExtractor::new();

    // Create a proof with redundancy
    let refl = ProofTerm::Reflexivity {
        term: Text::from("x"),
    };

    let axiom = ProofTerm::Axiom {
        name: Text::from("ax"),
        formula: Text::from("x = y"),
    };

    let trans = ProofTerm::Transitivity {
        left: Box::new(refl),
        right: Box::new(axiom.clone()),
    };

    // Extract should apply minimization if enabled
    // Since we can't directly test extract_proof with Z3 Dynamic,
    // we test the minimize_proof method directly
    let minimized = extractor.minimize_proof(&trans);

    assert_eq!(minimized, axiom);

    // Validate the minimized proof
    let validation = extractor.validate_proof(&minimized);
    assert!(validation.is_ok());
}

#[test]
fn test_proof_validation_cycle_detection() {
    // Note: Creating actual cycles in ProofTerm requires self-referential structures
    // which Rust prevents. This test verifies the validation logic handles depth limits.
    let extractor = ProofExtractor::new();

    // Create a deeply nested proof
    let mut proof = ProofTerm::Axiom {
        name: Text::from("base"),
        formula: Text::from("x = x"),
    };

    // Nest it many times
    for i in 0..100 {
        proof = ProofTerm::Lemma {
            conclusion: Text::from(format!("step_{}", i)),
            proof: Box::new(proof),
        };
    }

    let validation = extractor.validate_proof(&proof);

    // Should still be valid (under max depth of 1000)
    assert!(validation.is_valid);
}

// ==================== Export Tests ====================

#[test]
fn test_proof_export_smtlib2() {
    use verum_smt::ProofExporter;

    let axiom = ProofTerm::Axiom {
        name: Text::from("test_axiom"),
        formula: Text::from("(> x 0)"),
    };

    let smtlib = ProofExporter::to_smtlib2(&axiom);

    assert!(smtlib.contains("assert"));
    assert!(smtlib.contains("test_axiom"));
    assert!(smtlib.contains("(> x 0)"));
}

#[test]
fn test_proof_export_readable() {
    use verum_smt::ProofExporter;

    let axiom = ProofTerm::Axiom {
        name: Text::from("test"),
        formula: Text::from("x > 0"),
    };

    let readable = ProofExporter::to_readable(&axiom);

    assert!(readable.contains("Axiom"));
    assert!(readable.contains("test"));
    assert!(readable.contains("x > 0"));
}

// ==================== Complex Proof Tests ====================

#[test]
fn test_complex_proof_structure() {
    // Build a more complex proof structure
    let extractor = ProofExtractor::new();

    let ax1 = ProofTerm::Axiom {
        name: Text::from("ax1"),
        formula: Text::from("a = b"),
    };

    let ax2 = ProofTerm::Axiom {
        name: Text::from("ax2"),
        formula: Text::from("b = c"),
    };

    let trans = ProofTerm::Transitivity {
        left: Box::new(ax1),
        right: Box::new(ax2),
    };

    let sym = ProofTerm::Symmetry {
        equality: Box::new(trans),
    };

    // Validate the complex proof
    let validation = extractor.validate_proof(&sym);

    assert!(validation.is_valid);
    assert_eq!(sym.proof_depth(), 3);
    assert_eq!(sym.node_count(), 4);
}

#[test]
fn test_unit_resolution_validation() {
    let extractor = ProofExtractor::new();

    let clause1 = ProofTerm::Axiom {
        name: Text::from("c1"),
        formula: Text::from("A \\/ B"),
    };

    let clause2 = ProofTerm::Axiom {
        name: Text::from("c2"),
        formula: Text::from("~A \\/ C"),
    };

    let mut clauses = List::new();
    clauses.push(clause1);
    clauses.push(clause2);

    let unit_res = ProofTerm::UnitResolution { clauses };

    let validation = extractor.validate_proof(&unit_res);

    assert!(validation.is_valid);
}
