//! Demonstration of Coq and Lean proof export functionality
//!
//! This example shows how to convert ProofTerm structures into
//! valid Coq and Lean tactic syntax.
//!
//! Run with: cargo run --package verum_smt --example proof_export_demo

use verum_smt::proof_extraction::{ProofExporter, ProofTerm};
use verum_common::{Map, Text};

fn main() {
    println!("=== Verum SMT Proof Export Demo ===\n");

    // Example 1: Simple Axiom
    println!("Example 1: Axiom");
    println!("─────────────────");
    let axiom = ProofTerm::Axiom {
        name: "comm_law".into(),
        formula: "a + b = b + a".into(),
    };

    println!("Coq export:");
    println!("{}", ProofExporter::to_coq(&axiom).as_str());
    println!("\nLean export:");
    println!("{}\n", ProofExporter::to_lean(&axiom).as_str());

    // Example 2: Reflexivity
    println!("Example 2: Reflexivity");
    println!("─────────────────────");
    let reflexivity = ProofTerm::Reflexivity { term: "x".into() };

    println!("Coq export:");
    println!("{}", ProofExporter::to_coq(&reflexivity).as_str());
    println!("\nLean export:");
    println!("{}\n", ProofExporter::to_lean(&reflexivity).as_str());

    // Example 3: Symmetry
    println!("Example 3: Symmetry");
    println!("───────────────────");
    let equality = ProofTerm::Axiom {
        name: "eq_ab".into(),
        formula: "a = b".into(),
    };
    let symmetry = ProofTerm::Symmetry {
        equality: Box::new(equality),
    };

    println!("Coq export:");
    println!("{}", ProofExporter::to_coq(&symmetry).as_str());
    println!("\nLean export:");
    println!("{}\n", ProofExporter::to_lean(&symmetry).as_str());

    // Example 4: Transitivity
    println!("Example 4: Transitivity");
    println!("───────────────────────");
    let left_eq = ProofTerm::Axiom {
        name: "eq_ab".into(),
        formula: "a = b".into(),
    };
    let right_eq = ProofTerm::Axiom {
        name: "eq_bc".into(),
        formula: "b = c".into(),
    };
    let transitivity = ProofTerm::Transitivity {
        left: Box::new(left_eq),
        right: Box::new(right_eq),
    };

    println!("Coq export:");
    println!("{}", ProofExporter::to_coq(&transitivity).as_str());
    println!("\nLean export:");
    println!("{}\n", ProofExporter::to_lean(&transitivity).as_str());

    // Example 5: Modus Ponens
    println!("Example 5: Modus Ponens");
    println!("───────────────────────");
    let premise = ProofTerm::Axiom {
        name: "p".into(),
        formula: "P".into(),
    };
    let implication = ProofTerm::Axiom {
        name: "p_implies_q".into(),
        formula: "P => Q".into(),
    };
    let modus_ponens = ProofTerm::ModusPonens {
        premise: Box::new(premise),
        implication: Box::new(implication),
    };

    println!("Coq export:");
    println!("{}", ProofExporter::to_coq(&modus_ponens).as_str());
    println!("\nLean export:");
    println!("{}\n", ProofExporter::to_lean(&modus_ponens).as_str());

    // Example 6: Rewrite
    println!("Example 6: Rewrite");
    println!("──────────────────");
    let source = ProofTerm::Axiom {
        name: "hyp".into(),
        formula: "x + 0 = x".into(),
    };
    let rewrite = ProofTerm::Rewrite {
        source: Box::new(source),
        rule: "add_zero".into(),
        target: "x".into(),
    };

    println!("Coq export:");
    println!("{}", ProofExporter::to_coq(&rewrite).as_str());
    println!("\nLean export:");
    println!("{}\n", ProofExporter::to_lean(&rewrite).as_str());

    // Example 7: Theory Lemma
    println!("Example 7: Theory Lemma");
    println!("───────────────────────");
    let theory_lemma = ProofTerm::TheoryLemma {
        theory: "arithmetic".into(),
        lemma: "x + y = y + x".into(),
    };

    println!("Coq export:");
    println!("{}", ProofExporter::to_coq(&theory_lemma).as_str());
    println!("\nLean export:");
    println!("{}\n", ProofExporter::to_lean(&theory_lemma).as_str());

    // Example 8: Quantifier Instantiation
    println!("Example 8: Quantifier Instantiation");
    println!("────────────────────────────────────");
    let quantified = ProofTerm::Axiom {
        name: "forall_prop".into(),
        formula: "forall x. P(x)".into(),
    };
    let mut instantiation = Map::new();
    instantiation.insert("x".into(), "42".into());
    let quant_inst = ProofTerm::QuantifierInstantiation {
        quantified: Box::new(quantified),
        instantiation,
    };

    println!("Coq export:");
    println!("{}", ProofExporter::to_coq(&quant_inst).as_str());
    println!("\nLean export:");
    println!("{}\n", ProofExporter::to_lean(&quant_inst).as_str());

    // Example 9: Complex proof - Transitivity chain
    println!("Example 9: Complex Transitivity Chain");
    println!("──────────────────────────────────────");
    let eq1 = ProofTerm::Reflexivity { term: "a".into() };
    let eq2 = ProofTerm::Axiom {
        name: "eq_ab".into(),
        formula: "a = b".into(),
    };
    let trans_chain = ProofTerm::Transitivity {
        left: Box::new(eq1),
        right: Box::new(eq2),
    };

    println!("Coq export:");
    println!("{}", ProofExporter::to_coq(&trans_chain).as_str());
    println!("\nLean export:");
    println!("{}\n", ProofExporter::to_lean(&trans_chain).as_str());

    // Example 10: Lemma with nested proof
    println!("Example 10: Lemma");
    println!("─────────────────");
    let inner_proof = ProofTerm::Reflexivity { term: "x".into() };
    let lemma = ProofTerm::Lemma {
        conclusion: "x = x".into(),
        proof: Box::new(inner_proof),
    };

    println!("Coq export:");
    println!("{}", ProofExporter::to_coq(&lemma).as_str());
    println!("\nLean export:");
    println!("{}", ProofExporter::to_lean(&lemma).as_str());

    println!("\n=== Export Complete ===");
}
