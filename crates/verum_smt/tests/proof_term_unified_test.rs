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
//! Test suite for unified ProofTerm
//!
//! This test file demonstrates that the unified ProofTerm correctly:
//! 1. Supports all variants from all three sources
//! 2. Implements all required methods
//! 3. Provides conversions from the old types
//! 4. Maintains backward compatibility

// FIXED (Session 23): Tests enabled
// #![cfg(feature = "proof_term_unified_test_disabled")]

use verum_ast::{
    BinOp, Expr, ExprKind,
    literal::{IntLit, Literal, LiteralKind},
    span::Span,
};
use verum_smt::proof_term_unified::{ProofError, ProofTerm};
use verum_common::{Heap, List, Map, Text};

// ==================== Helper Functions ====================

fn dummy_expr() -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
        Span::dummy(),
    )
}

fn int_expr(n: u64) -> Expr {
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

fn eq_expr(left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::dummy(),
    )
}

// ==================== Basic Construction Tests ====================

#[test]
fn test_axiom_construction() {
    let proof = ProofTerm::axiom("commutativity", dummy_expr());

    match proof {
        ProofTerm::Axiom { name, .. } => {
            assert_eq!(name, Text::from("commutativity"));
        }
        _ => panic!("Expected Axiom variant"),
    }
}

#[test]
fn test_reflexivity_construction() {
    let term = int_expr(42);
    let proof = ProofTerm::reflexivity(term.clone());

    match proof {
        ProofTerm::Reflexivity { term: t } => {
            // Check that it's the same term
            assert!(matches!(t.kind, ExprKind::Literal(_)));
        }
        _ => panic!("Expected Reflexivity variant"),
    }
}

#[test]
fn test_smt_proof_construction() {
    let proof = ProofTerm::smt_proof("z3", dummy_expr());

    match proof {
        ProofTerm::SmtProof { solver, .. } => {
            assert_eq!(solver, Text::from("z3"));
        }
        _ => panic!("Expected SmtProof variant"),
    }
}

#[test]
fn test_lambda_construction() {
    let body = ProofTerm::axiom("base", dummy_expr());
    let proof = ProofTerm::lambda("x", body);

    match proof {
        ProofTerm::Lambda { var, .. } => {
            assert_eq!(var, Text::from("x"));
        }
        _ => panic!("Expected Lambda variant"),
    }
}

// ==================== Methods Tests ====================

#[test]
fn test_conclusion_axiom() {
    let formula = dummy_expr();
    let proof = ProofTerm::axiom("test", formula.clone());
    let conclusion = proof.conclusion();

    // Both should be true (dummy expr)
    match conclusion.kind {
        ExprKind::Literal(lit) => {
            assert!(matches!(lit.kind, LiteralKind::Bool(true)));
        }
        _ => panic!("Expected literal"),
    }
}

#[test]
fn test_conclusion_reflexivity() {
    let term = int_expr(42);
    let proof = ProofTerm::reflexivity(term.clone());
    let conclusion = proof.conclusion();

    // Should produce 42 = 42
    match conclusion.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(op, BinOp::Eq);
        }
        _ => panic!("Expected equality"),
    }
}

#[test]
fn test_used_axioms_single() {
    let proof = ProofTerm::axiom("axiom1", dummy_expr());
    let axioms = proof.used_axioms();

    assert_eq!(axioms.len(), 1);
    assert!(axioms.contains(&Text::from("axiom1")));
}

#[test]
fn test_used_axioms_nested() {
    let axiom1 = ProofTerm::axiom("axiom1", dummy_expr());
    let axiom2 = ProofTerm::axiom("axiom2", dummy_expr());
    let proof = ProofTerm::modus_ponens(axiom1, axiom2);

    let axioms = proof.used_axioms();

    assert_eq!(axioms.len(), 2);
    assert!(axioms.contains(&Text::from("axiom1")));
    assert!(axioms.contains(&Text::from("axiom2")));
}

#[test]
fn test_proof_depth_leaf() {
    let proof = ProofTerm::axiom("test", dummy_expr());
    assert_eq!(proof.proof_depth(), 1);
}

#[test]
fn test_proof_depth_nested() {
    let axiom1 = ProofTerm::axiom("axiom1", dummy_expr());
    let axiom2 = ProofTerm::axiom("axiom2", dummy_expr());
    let proof = ProofTerm::modus_ponens(axiom1, axiom2);

    // Should be 1 + max(1, 1) = 2
    assert_eq!(proof.proof_depth(), 2);
}

#[test]
fn test_node_count_single() {
    let proof = ProofTerm::axiom("test", dummy_expr());
    assert_eq!(proof.node_count(), 1);
}

#[test]
fn test_node_count_complex() {
    let axiom1 = ProofTerm::axiom("axiom1", dummy_expr());
    let axiom2 = ProofTerm::axiom("axiom2", dummy_expr());
    let proof = ProofTerm::modus_ponens(axiom1, axiom2);

    // Should be 1 + 1 + 1 = 3
    assert_eq!(proof.node_count(), 3);
}

#[test]
fn test_to_expr_classical() {
    let proof = ProofTerm::axiom("test", dummy_expr());
    let expr = proof.to_expr().unwrap();

    // Classical proofs return unit/true
    match expr.kind {
        ExprKind::Literal(lit) => {
            assert!(matches!(lit.kind, LiteralKind::Bool(true)));
        }
        _ => panic!("Expected bool literal"),
    }
}

#[test]
fn test_check_well_formed_valid() {
    let proof = ProofTerm::axiom("test", dummy_expr());
    assert!(proof.check_well_formed().is_ok());
}

#[test]
fn test_check_well_formed_invalid() {
    let proof = ProofTerm::axiom("", dummy_expr());
    assert!(proof.check_well_formed().is_err());
}

// ==================== All Variants Tests ====================

#[test]
fn test_all_variants_compile() {
    // Base cases
    let _ = ProofTerm::Axiom {
        name: "test".into(),
        formula: dummy_expr(),
    };

    let _ = ProofTerm::Assumption {
        id: 0,
        formula: dummy_expr(),
    };

    let _ = ProofTerm::Hypothesis {
        id: 0,
        formula: dummy_expr(),
    };

    // Classical logic
    let _ = ProofTerm::ModusPonens {
        premise: Heap::new(ProofTerm::axiom("p", dummy_expr())),
        implication: Heap::new(ProofTerm::axiom("i", dummy_expr())),
    };

    let _ = ProofTerm::Rewrite {
        source: Heap::new(ProofTerm::axiom("s", dummy_expr())),
        rule: "rule".into(),
        target: dummy_expr(),
    };

    let _ = ProofTerm::Symmetry {
        equality: Heap::new(ProofTerm::axiom("e", dummy_expr())),
    };

    let _ = ProofTerm::Transitivity {
        left: Heap::new(ProofTerm::axiom("l", dummy_expr())),
        right: Heap::new(ProofTerm::axiom("r", dummy_expr())),
    };

    let _ = ProofTerm::Reflexivity { term: dummy_expr() };

    // Theory reasoning
    let _ = ProofTerm::TheoryLemma {
        theory: "arithmetic".into(),
        lemma: dummy_expr(),
    };

    let _ = ProofTerm::UnitResolution {
        clauses: List::new(),
    };

    let _ = ProofTerm::QuantifierInstantiation {
        quantified: Heap::new(ProofTerm::axiom("q", dummy_expr())),
        instantiation: Map::new(),
    };

    // Constructive proofs
    let _ = ProofTerm::Lambda {
        var: "x".into(),
        body: Heap::new(ProofTerm::axiom("b", dummy_expr())),
    };

    let _ = ProofTerm::Cases {
        scrutinee: dummy_expr(),
        cases: List::new(),
    };

    let _ = ProofTerm::Induction {
        var: "n".into(),
        base_case: Heap::new(ProofTerm::axiom("base", dummy_expr())),
        inductive_case: Heap::new(ProofTerm::axiom("ind", dummy_expr())),
    };

    let _ = ProofTerm::Apply {
        rule: "rule".into(),
        premises: List::new(),
    };

    // SMT integration
    use verum_common::Maybe;
    let _ = ProofTerm::SmtProof {
        solver: "z3".into(),
        formula: dummy_expr(),
        smt_trace: Maybe::None,
    };

    // Dependent types
    let _ = ProofTerm::Subst {
        eq_proof: Heap::new(ProofTerm::axiom("eq", dummy_expr())),
        property: Heap::new(dummy_expr()),
    };

    // Meta-level
    let _ = ProofTerm::Lemma {
        conclusion: dummy_expr(),
        proof: Heap::new(ProofTerm::axiom("p", dummy_expr())),
    };
}

// ==================== Display Tests ====================

#[test]
fn test_display() {
    let proof = ProofTerm::axiom("test_axiom", dummy_expr());
    let display = format!("{}", proof);
    assert!(display.contains("Axiom"));
    assert!(display.contains("test_axiom"));
}

// ==================== Integration Tests ====================

#[test]
fn test_complex_proof_tree() {
    // Build a proof: (A -> B) ∧ A ⊢ B
    // 1. Axiom: A
    // 2. Axiom: A -> B
    // 3. Modus Ponens: 1, 2 ⊢ B

    let axiom_a = ProofTerm::axiom("A", dummy_expr());
    let axiom_impl = ProofTerm::axiom("A->B", dummy_expr());
    let proof_b = ProofTerm::modus_ponens(axiom_a, axiom_impl);

    // Check depth: 2 (MP + axioms at depth 1)
    assert_eq!(proof_b.proof_depth(), 2);

    // Check node count: 3 (1 MP + 2 axioms)
    assert_eq!(proof_b.node_count(), 3);

    // Check axioms used
    let axioms = proof_b.used_axioms();
    assert_eq!(axioms.len(), 2);
    assert!(axioms.contains(&Text::from("A")));
    assert!(axioms.contains(&Text::from("A->B")));
}

#[test]
fn test_equality_proof_chain() {
    // Build: a = b, b = c ⊢ a = c (transitivity)

    let a = int_expr(1);
    let b = int_expr(2);
    let c = int_expr(3);

    let eq_ab = ProofTerm::reflexivity(eq_expr(a.clone(), b.clone()));
    let eq_bc = ProofTerm::reflexivity(eq_expr(b.clone(), c.clone()));
    let eq_ac = ProofTerm::transitivity(eq_ab, eq_bc);

    // Check structure
    assert_eq!(eq_ac.proof_depth(), 2);
    assert_eq!(eq_ac.node_count(), 3);
}

// ==================== Error Handling Tests ====================

#[test]
fn test_extract_witness_from_non_existence_proof() {
    let proof = ProofTerm::axiom("not_existence", dummy_expr());
    let result = proof.extract_witness();

    assert!(result.is_err());
    match result {
        Err(ProofError::InvalidProof(_)) => {}
        _ => panic!("Expected InvalidProof error"),
    }
}

#[test]
fn test_well_formed_empty_unit_resolution() {
    let proof = ProofTerm::UnitResolution {
        clauses: List::new(),
    };

    let result = proof.check_well_formed();
    assert!(result.is_err());
}

// ==================== Convenience Constructor Tests ====================

#[test]
fn test_convenience_constructors() {
    // Test all convenience constructors work
    let _ = ProofTerm::axiom("test", dummy_expr());
    let _ = ProofTerm::assumption(0, dummy_expr());
    let _ = ProofTerm::reflexivity(dummy_expr());
    let _ = ProofTerm::smt_proof("z3", dummy_expr());
    let _ = ProofTerm::smt_proof_with_trace("z3", dummy_expr(), "trace".into());
    let _ = ProofTerm::theory_lemma("arithmetic", dummy_expr());
    let _ = ProofTerm::lambda("x", ProofTerm::axiom("body", dummy_expr()));
    let _ = ProofTerm::modus_ponens(
        ProofTerm::axiom("p", dummy_expr()),
        ProofTerm::axiom("i", dummy_expr()),
    );
    let _ = ProofTerm::transitivity(
        ProofTerm::axiom("l", dummy_expr()),
        ProofTerm::axiom("r", dummy_expr()),
    );
}
