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
// Tests for refinement module
// Migrated from src/refinement.rs per CLAUDE.md standards

use verum_smt::refinement::*;
use verum_smt::verify::VerifyMode;

use verum_ast::span::Span;
use verum_ast::{Type, TypeKind};

// Helper to create Int type
fn int_type() -> Type {
    Type::new(TypeKind::Int, Span::dummy())
}

#[test]
fn test_runtime_mode_skips_smt() {
    let _verifier = RefinementVerifier::with_mode(VerifyMode::Runtime);

    // Create a refinement type: Int{> 0}
    // (Simplified for test - actual construction would be more complex)

    // In runtime mode, verification should succeed immediately
    // without invoking SMT
}

#[test]
fn test_complexity_categorization() {
    // Simple types should be categorized as Simple
    let ty = int_type();
    let category = categorize_complexity(&ty);
    assert_eq!(category, PredicateComplexity::Simple);
}

#[test]
fn test_is_refinement_type() {
    let ty = int_type();
    assert!(!is_refinement_type(&ty));

    // Would need to construct an actual refinement type to test positive case
}

// ==================== Proof Extraction Integration Tests ====================

use verum_smt::proof_extraction::{ProofExtractor, ProofTerm, ProofValidation};
use verum_smt::z3_backend::ProofWitness;
use verum_common::{List, Set, Text};

#[test]
fn test_validate_proof_witness_empty() {
    let verifier = RefinementVerifier::new();

    // Empty proof term should be invalid
    let witness = ProofWitness {
        proof_term: Text::from(""),
        used_axioms: Set::new(),
        proof_steps: 0,
    };

    let validation = verifier.validate_proof_witness(&witness);
    assert!(!validation.is_valid);
    assert!(!validation.errors.is_empty());
    assert!(validation.errors[0].contains("empty"));
}

#[test]
fn test_validate_proof_witness_valid() {
    let verifier = RefinementVerifier::new();

    // Valid proof witness
    let mut axioms = Set::new();
    axioms.insert(Text::from("axiom1"));

    let witness = ProofWitness {
        proof_term: Text::from("(proof (asserted x > 0))"),
        used_axioms: axioms,
        proof_steps: 5,
    };

    let validation = verifier.validate_proof_witness(&witness);
    assert!(validation.is_valid);
    assert!(validation.errors.is_empty());
}

#[test]
fn test_validate_proof_witness_with_warnings() {
    let verifier = RefinementVerifier::new();

    // Proof with no steps or axioms should produce warnings
    let witness = ProofWitness {
        proof_term: Text::from("(proof)"),
        used_axioms: Set::new(),
        proof_steps: 0,
    };

    let validation = verifier.validate_proof_witness(&witness);
    // Valid but with warnings about missing steps and axioms
    assert!(validation.is_valid);
    assert!(!validation.warnings.is_empty());
}

#[test]
fn test_validate_proof_witness_exceeds_complexity() {
    let verifier = RefinementVerifier::new();

    // Proof with too many steps should produce warnings
    let mut axioms = Set::new();
    axioms.insert(Text::from("axiom1"));

    let witness = ProofWitness {
        proof_term: Text::from("(proof)"),
        used_axioms: axioms,
        proof_steps: 150_000, // Exceeds MAX_REASONABLE_PROOF_STEPS
    };

    let validation = verifier.validate_proof_witness(&witness);
    assert!(validation.is_valid);
    // Should have a warning about exceeding step limit
    assert!(
        validation
            .warnings
            .iter()
            .any(|w| w.contains("exceeds") || w.contains("limit"))
    );
}

#[test]
fn test_validate_proof_witness_unknown_rules() {
    let verifier = RefinementVerifier::new();

    // Proof with unknown rules should produce warning
    let witness = ProofWitness {
        proof_term: Text::from("(proof (unknown: rule))"),
        used_axioms: Set::new(),
        proof_steps: 1,
    };

    let validation = verifier.validate_proof_witness(&witness);
    assert!(validation.is_valid);
    // Should have warning about unknown rules
    assert!(validation.warnings.iter().any(|w| w.contains("unknown")));
}

#[test]
fn test_proof_extractor_integration() {
    // Test that ProofExtractor can parse and analyze proof terms
    let extractor = ProofExtractor::new();

    // Create a simple axiom proof term
    let axiom = ProofTerm::Axiom {
        name: Text::from("test_axiom"),
        formula: Text::from("x > 0"),
    };

    // Analyze the proof
    let analysis = extractor.analyze(&axiom);
    assert_eq!(analysis.depth, 1);
    assert_eq!(analysis.node_count, 1);
    assert!(analysis.axioms_used.contains(&Text::from("test_axiom")));

    // Validate the proof
    let validation = extractor.validate_proof(&axiom);
    assert!(validation.is_valid);
}

#[test]
fn test_proof_extractor_modus_ponens() {
    let extractor = ProofExtractor::new();

    // Create a modus ponens proof: from A and (A => B), derive B
    let premise = ProofTerm::Axiom {
        name: Text::from("A"),
        formula: Text::from("A"),
    };

    let implication = ProofTerm::Axiom {
        name: Text::from("A_implies_B"),
        formula: Text::from("A => B"),
    };

    let mp = ProofTerm::ModusPonens {
        premise: Box::new(premise),
        implication: Box::new(implication),
    };

    // Analyze the proof
    let analysis = extractor.analyze(&mp);
    assert_eq!(analysis.depth, 2); // MP + its children
    assert_eq!(analysis.node_count, 3); // MP + 2 axioms
    assert!(analysis.axioms_used.contains(&Text::from("A")));
    assert!(analysis.axioms_used.contains(&Text::from("A_implies_B")));

    // Validate the proof
    let validation = extractor.validate_proof(&mp);
    assert!(validation.is_valid);
}

#[test]
fn test_proof_extractor_transitivity() {
    let extractor = ProofExtractor::new();

    // Create a transitivity proof: from (x = y) and (y = z), derive (x = z)
    let left = ProofTerm::Axiom {
        name: Text::from("x_eq_y"),
        formula: Text::from("x = y"),
    };

    let right = ProofTerm::Axiom {
        name: Text::from("y_eq_z"),
        formula: Text::from("y = z"),
    };

    let trans = ProofTerm::Transitivity {
        left: Box::new(left),
        right: Box::new(right),
    };

    let analysis = extractor.analyze(&trans);
    assert_eq!(analysis.depth, 2);
    assert_eq!(analysis.node_count, 3);

    let validation = extractor.validate_proof(&trans);
    assert!(validation.is_valid);
}

#[test]
fn test_extract_and_validate_proof() {
    let verifier = RefinementVerifier::new();
    let mut proof_result = verum_smt::verify::ProofResult::new(
        verum_smt::cost::VerificationCost::new("test".into(), std::time::Duration::ZERO, true),
    );

    // No witness initially
    let result = verifier.extract_and_validate_proof(&mut proof_result);
    assert!(result.is_none());

    // Add a witness
    let mut axioms = Set::new();
    axioms.insert(Text::from("test_axiom"));

    let witness = ProofWitness {
        proof_term: Text::from("(proof (asserted x > 0))"),
        used_axioms: axioms,
        proof_steps: 3,
    };

    proof_result = proof_result.with_proof_witness(witness);

    // Now extraction should succeed
    let result = verifier.extract_and_validate_proof(&mut proof_result);
    assert!(result.is_some());
    assert!(result.unwrap().is_valid);
}

#[test]
fn test_proof_validation_summary() {
    let validation = ProofValidation {
        is_valid: true,
        errors: List::new(),
        warnings: List::new(),
    };
    let summary = validation.summary();
    assert!(summary.contains("valid"));

    let mut warnings = List::new();
    warnings.push(Text::from("warning 1"));
    let validation_with_warnings = ProofValidation {
        is_valid: true,
        errors: List::new(),
        warnings,
    };
    let summary = validation_with_warnings.summary();
    assert!(summary.contains("warnings") || summary.contains("warning"));

    let mut errors = List::new();
    errors.push(Text::from("error 1"));
    let invalid_validation = ProofValidation {
        is_valid: false,
        errors,
        warnings: List::new(),
    };
    let summary = invalid_validation.summary();
    assert!(summary.contains("invalid") || summary.contains("error"));
}
