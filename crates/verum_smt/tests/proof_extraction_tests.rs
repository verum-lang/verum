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
// Tests for proof_extraction module
// Migrated from src/proof_extraction.rs per CLAUDE.md standards
//
// FIXED (Session 23): Added explicit Text::from() calls for type inference.

use verum_smt::proof_extraction::*;
use verum_common::{List, Text};

#[test]
fn test_proof_term_axiom() {
    let axiom = ProofTerm::Axiom {
        name: Text::from("ax1"),
        formula: Text::from("x > 0"),
    };

    assert_eq!(axiom.conclusion(), Text::from("x > 0"));
    assert_eq!(axiom.proof_depth(), 1);
    assert_eq!(axiom.node_count(), 1);

    let axioms = axiom.used_axioms();
    assert_eq!(axioms.len(), 1);
    assert!(axioms.contains(&Text::from("ax1")));
}

#[test]
fn test_proof_term_reflexivity() {
    let refl = ProofTerm::Reflexivity {
        term: Text::from("x"),
    };

    assert_eq!(refl.conclusion(), Text::from("x = x"));
    assert_eq!(refl.proof_depth(), 1);
    assert_eq!(refl.node_count(), 1);
}

#[test]
fn test_proof_term_modus_ponens() {
    let premise = ProofTerm::Axiom {
        name: Text::from("premise"),
        formula: Text::from("A"),
    };

    let implication = ProofTerm::Axiom {
        name: Text::from("implication"),
        formula: Text::from("A => B"),
    };

    let mp = ProofTerm::ModusPonens {
        premise: Box::new(premise),
        implication: Box::new(implication),
    };

    assert_eq!(mp.proof_depth(), 2);
    assert_eq!(mp.node_count(), 3);

    let axioms = mp.used_axioms();
    assert_eq!(axioms.len(), 2);
}

#[test]
fn test_proof_extractor() {
    let extractor = ProofExtractor::new();
    assert!(extractor.simplify_proofs);
    assert_eq!(extractor.max_depth, 1000);
}

#[test]
fn test_proof_analysis() {
    let axiom = ProofTerm::Axiom {
        name: Text::from("ax1"),
        formula: Text::from("x > 0"),
    };

    let extractor = ProofExtractor::new();
    let analysis = extractor.analyze(&axiom);

    assert_eq!(analysis.depth, 1);
    assert_eq!(analysis.node_count, 1);
    assert!(!analysis.has_quantifiers);
    assert_eq!(analysis.theory_lemmas, 0);
}

#[test]
fn test_proof_export_smtlib2() {
    let axiom = ProofTerm::Axiom {
        name: Text::from("ax1"),
        formula: Text::from("(> x 0)"),
    };

    let smtlib = ProofExporter::to_smtlib2(&axiom);
    assert!(smtlib.contains("assert"));
    assert!(smtlib.contains("ax1"));
}

#[test]
fn test_proof_minimizer() {
    // Create redundant transitivity with reflexivity
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
fn test_proof_validation_valid() {
    let extractor = ProofExtractor::new();

    let axiom = ProofTerm::Axiom {
        name: Text::from("test_axiom"),
        formula: Text::from("x > 0"),
    };

    let validation = extractor.validate_proof(&axiom);

    assert!(validation.is_ok());
    assert!(validation.is_valid);
    assert_eq!(validation.errors.len(), 0);
}

#[test]
fn test_proof_validation_invalid_empty_formula() {
    let extractor = ProofExtractor::new();

    let axiom = ProofTerm::Axiom {
        name: Text::from("test"),
        formula: Text::from(""),
    };

    let validation = extractor.validate_proof(&axiom);

    assert!(!validation.is_ok());
    assert!(!validation.is_valid);
    assert!(!validation.errors.is_empty());
    assert!(validation.errors[0].contains("empty formula"));
}

#[test]
fn test_proof_validation_symmetry() {
    let extractor = ProofExtractor::new();

    // Valid symmetry proof
    let equality = ProofTerm::Axiom {
        name: Text::from("eq"),
        formula: Text::from("x = y"),
    };

    let sym = ProofTerm::Symmetry {
        equality: Box::new(equality),
    };

    let validation = extractor.validate_proof(&sym);
    assert!(validation.is_valid);

    // Invalid symmetry (not an equality)
    let non_equality = ProofTerm::Axiom {
        name: Text::from("ne"),
        formula: Text::from("x > y"),
    };

    let bad_sym = ProofTerm::Symmetry {
        equality: Box::new(non_equality),
    };

    let validation2 = extractor.validate_proof(&bad_sym);
    assert!(!validation2.is_valid);
}

#[test]
fn test_proof_formatter() {
    let formatter = ProofFormatter;

    let axiom = ProofTerm::Axiom {
        name: Text::from("test"),
        formula: Text::from("x > 0"),
    };

    let formatted = formatter.format(&axiom);
    assert!(formatted.contains("Axiom"));
    assert!(formatted.contains("test"));
    assert!(formatted.contains("x > 0"));
}

#[test]
fn test_proof_formatter_nested() {
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
fn test_extract_and_minimize() {
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

    // Minimize should remove reflexivity
    let minimized = extractor.minimize_proof(&trans);
    assert_eq!(minimized, axiom);
}

#[test]
fn test_validation_summary() {
    let validation = ProofValidation {
        is_valid: true,
        errors: List::new(),
        warnings: List::new(),
    };

    let summary = validation.summary();
    assert!(summary.contains("valid"));

    let validation_with_warnings = ProofValidation {
        is_valid: true,
        errors: List::new(),
        warnings: {
            let mut w = List::new();
            w.push(Text::from("Warning 1"));
            w
        },
    };

    let summary2 = validation_with_warnings.summary();
    assert!(summary2.contains("warnings"));

    let invalid = ProofValidation {
        is_valid: false,
        errors: {
            let mut e = List::new();
            e.push(Text::from("Error 1"));
            e
        },
        warnings: List::new(),
    };

    let summary3 = invalid.summary();
    assert!(summary3.contains("invalid"));
}

// ==================== New Proof Rule Tests ====================

#[test]
fn test_proof_term_and_elim() {
    let conjunction = ProofTerm::Axiom {
        name: Text::from("conj"),
        formula: Text::from("(and A B)"),
    };

    let and_elim = ProofTerm::AndElim {
        conjunction: Box::new(conjunction),
        index: 0,
        result: Text::from("A"),
    };

    assert_eq!(and_elim.conclusion(), Text::from("A"));
    assert_eq!(and_elim.proof_depth(), 2);
    assert_eq!(and_elim.node_count(), 2);

    let axioms = and_elim.used_axioms();
    assert_eq!(axioms.len(), 1);
    assert!(axioms.contains(&Text::from("conj")));
}

#[test]
fn test_proof_term_not_or_elim() {
    let negated_disjunction = ProofTerm::Axiom {
        name: Text::from("neg_disj"),
        formula: Text::from("(not (or A B))"),
    };

    let not_or_elim = ProofTerm::NotOrElim {
        negated_disjunction: Box::new(negated_disjunction),
        index: 1,
        result: Text::from("(not B)"),
    };

    assert_eq!(not_or_elim.conclusion(), Text::from("(not B)"));
    assert_eq!(not_or_elim.proof_depth(), 2);
    assert_eq!(not_or_elim.node_count(), 2);
}

#[test]
fn test_proof_term_iff_true() {
    let proof_of_p = ProofTerm::Axiom {
        name: Text::from("p_true"),
        formula: Text::from("P"),
    };

    let iff_true = ProofTerm::IffTrue {
        proof: Box::new(proof_of_p),
        formula: Text::from("P"),
    };

    assert_eq!(iff_true.conclusion(), Text::from("(iff P true)"));
    assert_eq!(iff_true.proof_depth(), 2);
    assert_eq!(iff_true.node_count(), 2);
}

#[test]
fn test_proof_term_iff_false() {
    let proof_of_not_p = ProofTerm::Axiom {
        name: Text::from("not_p"),
        formula: Text::from("(not P)"),
    };

    let iff_false = ProofTerm::IffFalse {
        proof: Box::new(proof_of_not_p),
        formula: Text::from("P"),
    };

    assert_eq!(iff_false.conclusion(), Text::from("(iff P false)"));
    assert_eq!(iff_false.proof_depth(), 2);
}

#[test]
fn test_proof_term_commutativity() {
    let comm = ProofTerm::Commutativity {
        left: Text::from("(+ a b)"),
        right: Text::from("(+ b a)"),
    };

    assert_eq!(comm.conclusion(), Text::from("(= (+ a b) (+ b a))"));
    assert_eq!(comm.proof_depth(), 1);
    assert_eq!(comm.node_count(), 1);

    // Commutativity has no sub-proofs, so no axioms
    let axioms = comm.used_axioms();
    assert_eq!(axioms.len(), 0);
}

#[test]
fn test_proof_term_monotonicity() {
    let premise1 = ProofTerm::Axiom {
        name: Text::from("eq1"),
        formula: Text::from("a = a'"),
    };
    let premise2 = ProofTerm::Axiom {
        name: Text::from("eq2"),
        formula: Text::from("b = b'"),
    };

    let mono = ProofTerm::Monotonicity {
        premises: vec![premise1, premise2].into(),
        conclusion: Text::from("(f a b) = (f a' b')"),
    };

    assert_eq!(mono.conclusion(), Text::from("(f a b) = (f a' b')"));
    assert_eq!(mono.proof_depth(), 2);
    assert_eq!(mono.node_count(), 3);

    let axioms = mono.used_axioms();
    assert_eq!(axioms.len(), 2);
}

#[test]
fn test_proof_term_distributivity() {
    let dist = ProofTerm::Distributivity {
        formula: Text::from("(= (* a (+ b c)) (+ (* a b) (* a c)))"),
    };

    assert_eq!(dist.proof_depth(), 1);
    assert_eq!(dist.node_count(), 1);
    assert!(dist.used_axioms().is_empty());
}

#[test]
fn test_proof_term_def_axiom() {
    let def_ax = ProofTerm::DefAxiom {
        formula: Text::from("(or (not (and p q)) p)"),
    };

    assert_eq!(def_ax.conclusion(), Text::from("(or (not (and p q)) p)"));
    assert_eq!(def_ax.proof_depth(), 1);
}

#[test]
fn test_proof_term_def_intro() {
    let def_intro = ProofTerm::DefIntro {
        name: Text::from("n"),
        definition: Text::from("(= n (+ x y))"),
    };

    assert_eq!(def_intro.conclusion(), Text::from("(= n (+ x y))"));
    assert_eq!(def_intro.proof_depth(), 1);
}

#[test]
fn test_proof_term_apply_def() {
    let def_proof = ProofTerm::DefIntro {
        name: Text::from("n"),
        definition: Text::from("(= n (+ x y))"),
    };

    let apply_def = ProofTerm::ApplyDef {
        def_proof: Box::new(def_proof),
        original: Text::from("(+ x y)"),
        name: Text::from("n"),
    };

    assert_eq!(apply_def.conclusion(), Text::from("n"));
    assert_eq!(apply_def.proof_depth(), 2);
}

#[test]
fn test_proof_term_iff_oeq() {
    let iff_proof = ProofTerm::Axiom {
        name: Text::from("iff"),
        formula: Text::from("(iff p q)"),
    };

    let iff_oeq = ProofTerm::IffOEq {
        iff_proof: Box::new(iff_proof),
        left: Text::from("p"),
        right: Text::from("q"),
    };

    assert_eq!(iff_oeq.conclusion(), Text::from("(~ p q)"));
    assert_eq!(iff_oeq.proof_depth(), 2);
}

#[test]
fn test_proof_term_nnf_pos() {
    let premise = ProofTerm::Axiom {
        name: Text::from("eq"),
        formula: Text::from("s ~ r"),
    };

    let nnf_pos = ProofTerm::NNFPos {
        premises: vec![premise].into(),
        conclusion: Text::from("(implies s_1 ...) ~ (or r_1 ...)"),
    };

    assert_eq!(nnf_pos.proof_depth(), 2);
    assert_eq!(nnf_pos.node_count(), 2);
}

#[test]
fn test_proof_term_nnf_neg() {
    let premise = ProofTerm::Axiom {
        name: Text::from("neq"),
        formula: Text::from("(not s) ~ r"),
    };

    let nnf_neg = ProofTerm::NNFNeg {
        premises: vec![premise].into(),
        conclusion: Text::from("(not (and s ...)) ~ (or r ...)"),
    };

    assert_eq!(nnf_neg.proof_depth(), 2);
}

#[test]
fn test_proof_term_skolemize() {
    let skolem = ProofTerm::Skolemize {
        formula: Text::from("(~ (exists x (p x y)) (p (sk y) y))"),
    };

    assert_eq!(
        skolem.conclusion(),
        Text::from("(~ (exists x (p x y)) (p (sk y) y))")
    );
    assert_eq!(skolem.proof_depth(), 1);
}

#[test]
fn test_proof_term_quant_intro() {
    let body_proof = ProofTerm::Axiom {
        name: Text::from("body_eq"),
        formula: Text::from("(~ p q)"),
    };

    let quant_intro = ProofTerm::QuantIntro {
        body_proof: Box::new(body_proof),
        conclusion: Text::from("(~ (forall (x) p) (forall (x) q))"),
    };

    assert_eq!(
        quant_intro.conclusion(),
        Text::from("(~ (forall (x) p) (forall (x) q))")
    );
    assert_eq!(quant_intro.proof_depth(), 2);

    // Should detect quantifier presence
    let extractor = ProofExtractor::new();
    let analysis = extractor.analyze(&quant_intro);
    assert!(analysis.has_quantifiers);
}

#[test]
fn test_proof_term_bind() {
    let body_proof = ProofTerm::Axiom {
        name: Text::from("f"),
        formula: Text::from("f(x)"),
    };

    let bind = ProofTerm::Bind {
        body_proof: Box::new(body_proof),
        variables: vec![Text::from("x")].into(),
        conclusion: Text::from("(forall (x) f(x))"),
    };

    assert_eq!(bind.conclusion(), Text::from("(forall (x) f(x))"));
    assert_eq!(bind.proof_depth(), 2);

    let extractor = ProofExtractor::new();
    let analysis = extractor.analyze(&bind);
    assert!(analysis.has_quantifiers);
}

#[test]
fn test_proof_term_pull_quant() {
    let pull = ProofTerm::PullQuant {
        formula: Text::from("(iff (f (forall (x) q(x)) r) (forall (x) (f (q x) r)))"),
    };

    assert_eq!(pull.proof_depth(), 1);

    let extractor = ProofExtractor::new();
    let analysis = extractor.analyze(&pull);
    assert!(analysis.has_quantifiers);
}

#[test]
fn test_proof_term_push_quant() {
    let push = ProofTerm::PushQuant {
        formula: Text::from("(iff (forall (x) (and p q)) (and (forall (x) p) (forall (x) q)))"),
    };

    assert_eq!(push.proof_depth(), 1);

    let extractor = ProofExtractor::new();
    let analysis = extractor.analyze(&push);
    assert!(analysis.has_quantifiers);
}

#[test]
fn test_proof_term_elim_unused_vars() {
    let elim = ProofTerm::ElimUnusedVars {
        formula: Text::from("(iff (forall (x y) p[x]) (forall (x) p[x]))"),
    };

    assert_eq!(elim.proof_depth(), 1);

    let extractor = ProofExtractor::new();
    let analysis = extractor.analyze(&elim);
    assert!(analysis.has_quantifiers);
}

#[test]
fn test_proof_term_destructive_eq_res() {
    let der = ProofTerm::DestructiveEqRes {
        formula: Text::from("(iff (forall (x) (or (not (= x t)) P[x])) P[t])"),
    };

    assert_eq!(der.proof_depth(), 1);
}

#[test]
fn test_proof_term_hyper_resolve() {
    let clause1 = ProofTerm::Axiom {
        name: Text::from("c1"),
        formula: Text::from("(or l1 l2)"),
    };
    let clause2 = ProofTerm::Axiom {
        name: Text::from("c2"),
        formula: Text::from("(not l1)"),
    };

    let hyper = ProofTerm::HyperResolve {
        clauses: vec![clause1, clause2].into(),
        conclusion: Text::from("l2"),
    };

    assert_eq!(hyper.conclusion(), Text::from("l2"));
    assert_eq!(hyper.proof_depth(), 2);
    assert_eq!(hyper.node_count(), 3);
}

#[test]
fn test_proof_formatter_new_rules() {
    let formatter = ProofFormatter;

    // Test AndElim formatting
    let conjunction = ProofTerm::Axiom {
        name: Text::from("conj"),
        formula: Text::from("(and A B)"),
    };
    let and_elim = ProofTerm::AndElim {
        conjunction: Box::new(conjunction),
        index: 0,
        result: Text::from("A"),
    };
    let formatted = formatter.format(&and_elim);
    assert!(formatted.contains("And Elim"));

    // Test Commutativity formatting
    let comm = ProofTerm::Commutativity {
        left: Text::from("(+ a b)"),
        right: Text::from("(+ b a)"),
    };
    let formatted_comm = formatter.format(&comm);
    assert!(formatted_comm.contains("Commutativity"));

    // Test Skolemize formatting
    let skolem = ProofTerm::Skolemize {
        formula: Text::from("(~ (exists x p) (p sk))"),
    };
    let formatted_skolem = formatter.format(&skolem);
    assert!(formatted_skolem.contains("Skolemize"));
}

#[test]
fn test_proof_minimizer_new_rules() {
    // Test minimization with new proof rules
    let conj = ProofTerm::Axiom {
        name: Text::from("conj"),
        formula: Text::from("(and A B)"),
    };

    let and_elim = ProofTerm::AndElim {
        conjunction: Box::new(conj.clone()),
        index: 0,
        result: Text::from("A"),
    };

    // Minimization should preserve the structure (no redundancy)
    let minimized = ProofMinimizer::minimize(&and_elim);
    assert_eq!(minimized.node_count(), 2);
}

#[test]
fn test_proof_validation_new_rules() {
    let extractor = ProofExtractor::new();

    // Valid AndElim
    let conj = ProofTerm::Axiom {
        name: Text::from("conj"),
        formula: Text::from("(and A B)"),
    };
    let and_elim = ProofTerm::AndElim {
        conjunction: Box::new(conj),
        index: 0,
        result: Text::from("A"),
    };
    let validation = extractor.validate_proof(&and_elim);
    assert!(validation.is_valid);

    // Valid Commutativity
    let comm = ProofTerm::Commutativity {
        left: Text::from("(+ a b)"),
        right: Text::from("(+ b a)"),
    };
    let validation_comm = extractor.validate_proof(&comm);
    assert!(validation_comm.is_valid);

    // Invalid - empty result
    let invalid_and_elim = ProofTerm::AndElim {
        conjunction: Box::new(ProofTerm::Axiom {
            name: Text::from("c"),
            formula: Text::from("(and A B)"),
        }),
        index: 0,
        result: Text::from(""),
    };
    let invalid_validation = extractor.validate_proof(&invalid_and_elim);
    assert!(!invalid_validation.is_valid);
    assert!(
        invalid_validation
            .errors
            .iter()
            .any(|e| e.contains("AndElim"))
    );
}
