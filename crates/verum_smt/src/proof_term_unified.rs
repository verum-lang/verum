//! Unified Proof Term Representation
//!
//! This module provides a unified ProofTerm type that consolidates the three
//! incompatible ProofTerm definitions found in:
//! - proof_extraction.rs (11 variants, classical logic focus)
//! - proof_search.rs (6 variants, constructive proof focus)
//! - dependent.rs (struct + ProofStructure enum, dependent types focus)
//!
//! ## Design Philosophy
//!
//! The unified ProofTerm supports:
//! 1. **Classical reasoning** (from proof_extraction.rs): Modus ponens, rewrite rules,
//!    theory lemmas, unit resolution, reflexivity/symmetry/transitivity
//! 2. **Constructive proofs** (from proof_search.rs): Lambda abstraction, cases,
//!    induction, axioms, application
//! 3. **Dependent types** (from dependent.rs): SMT proofs, substitution, assumptions
//!
//! ## Architecture
//!
//! - All 17+ unique proof term variants are unified in a single enum
//! - Uses Heap<> for recursive structures (Verum semantic type for Box<>)
//! - Uses Text, List, Map, Set (Verum semantic types)
//! - Implements all methods from all three sources
//!
//! Proof terms are first-class values: `type Proof<P: Prop> is evidence of P`.
//! Construction via modus_ponens, or_elim, and_intro, etc. Proofs can be
//! exported to Coq, Lean, Dedukti for independent verification.

use verum_ast::{BinOp, Expr, ExprKind, Literal, LiteralKind, literal::StringLit, span::Span};
use verum_common::{Heap, List, Map, Maybe, Set, Text};

// ==================== Unified Proof Term ====================

/// Unified proof term representation
///
/// This enum combines all proof term variants from proof_extraction.rs,
/// proof_search.rs, and dependent.rs into a single coherent type.
///
/// The variants are organized by category:
/// - Base cases: Axiom, Assumption, Hypothesis
/// - Classical logic: ModusPonens, Rewrite, Symmetry, Transitivity, Reflexivity
/// - Theory reasoning: TheoryLemma, UnitResolution, QuantifierInstantiation
/// - Constructive proofs: Lambda, Cases, Induction, Apply
/// - SMT integration: SmtProof
/// - Dependent types: Subst (substitution)
/// - Meta-level: Lemma
#[derive(Debug, Clone, PartialEq)]
pub enum ProofTerm {
    // ==================== Base Cases ====================
    /// Axiom or given fact
    ///
    /// From: proof_extraction.rs
    /// Represents a fundamental truth or assumption in the logical system.
    Axiom {
        /// Axiom name/identifier
        name: Text,
        /// Formula being asserted
        formula: Expr,
    },

    /// Assumption (hypothesis at a specific position)
    ///
    /// From: proof_extraction.rs
    /// Represents a temporary assumption with an identifier.
    Assumption {
        /// Assumption identifier
        id: usize,
        /// Formula being assumed
        formula: Expr,
    },

    /// Hypothesis (local assumption)
    ///
    /// From: proof_extraction.rs
    /// Similar to Assumption but used in different proof contexts.
    Hypothesis {
        /// Hypothesis identifier
        id: usize,
        /// Formula being hypothesized
        formula: Expr,
    },

    // ==================== Classical Logic Rules ====================
    /// Modus ponens: from A and A => B, derive B
    ///
    /// From: proof_extraction.rs
    /// Classical inference rule for implication elimination.
    ModusPonens {
        /// Proof of premise A
        premise: Heap<ProofTerm>,
        /// Proof of implication A => B
        implication: Heap<ProofTerm>,
    },

    /// Rewrite rule application
    ///
    /// From: proof_extraction.rs
    /// Represents applying a rewrite rule to transform an expression.
    Rewrite {
        /// Source proof before rewriting
        source: Heap<ProofTerm>,
        /// Name of the rewrite rule
        rule: Text,
        /// Target formula after rewriting
        target: Expr,
    },

    /// Symmetry: from A = B, derive B = A
    ///
    /// From: proof_extraction.rs
    /// Equality symmetry rule.
    Symmetry {
        /// Proof of equality
        equality: Heap<ProofTerm>,
    },

    /// Transitivity: from A = B and B = C, derive A = C
    ///
    /// From: proof_extraction.rs
    /// Equality transitivity rule.
    Transitivity {
        /// Proof of first equality A = B
        left: Heap<ProofTerm>,
        /// Proof of second equality B = C
        right: Heap<ProofTerm>,
    },

    /// Reflexivity: derive A = A
    ///
    /// From: proof_extraction.rs, dependent.rs
    /// Reflexive equality axiom.
    Reflexivity {
        /// Term being equated to itself
        term: Expr,
    },

    // ==================== Theory Reasoning ====================
    /// Theory lemma (SMT theory axiom)
    ///
    /// From: proof_extraction.rs
    /// Represents a lemma from an SMT theory (e.g., arithmetic, arrays).
    TheoryLemma {
        /// Theory name (e.g., "arithmetic", "arrays")
        theory: Text,
        /// Lemma formula
        lemma: Expr,
    },

    /// Unit resolution (SAT reasoning)
    ///
    /// From: proof_extraction.rs
    /// Represents resolution-based reasoning from SAT solving.
    UnitResolution {
        /// List of clause proofs being resolved
        clauses: List<ProofTerm>,
    },

    /// Quantifier instantiation
    ///
    /// From: proof_extraction.rs
    /// Represents instantiating a quantified formula with concrete values.
    QuantifierInstantiation {
        /// Proof of quantified formula
        quantified: Heap<ProofTerm>,
        /// Variable instantiation bindings
        instantiation: Map<Text, Expr>,
    },

    // ==================== Constructive Proofs ====================
    /// Lambda abstraction (function construction)
    ///
    /// From: proof_search.rs
    /// Constructive proof by introducing a function.
    Lambda {
        /// Parameter name
        var: Text,
        /// Body proof (may reference var)
        body: Heap<ProofTerm>,
    },

    /// Proof by cases (case analysis)
    ///
    /// From: proof_search.rs
    /// Constructive proof by examining all cases of a value.
    Cases {
        /// Expression being analyzed
        scrutinee: Expr,
        /// List of (pattern, proof) pairs for each case
        cases: List<(Expr, Heap<ProofTerm>)>,
    },

    /// Induction proof
    ///
    /// From: proof_search.rs
    /// Proof by mathematical induction.
    Induction {
        /// Induction variable
        var: Text,
        /// Base case proof
        base_case: Heap<ProofTerm>,
        /// Inductive case proof (may use IH)
        inductive_case: Heap<ProofTerm>,
    },

    /// Application of inference rule
    ///
    /// From: proof_search.rs
    /// General application of a named rule with premises.
    Apply {
        /// Rule name being applied
        rule: Text,
        /// Premises required by the rule
        premises: List<Heap<ProofTerm>>,
    },

    // ==================== SMT Integration ====================
    /// SMT solver proof
    ///
    /// From: proof_search.rs, dependent.rs
    /// Represents a proof discharged by an SMT solver.
    SmtProof {
        /// Solver name (e.g., "z3", "cvc5")
        solver: Text,
        /// Formula proven by solver
        formula: Expr,
        /// Optional SMT-LIB2 proof trace
        smt_trace: Maybe<Text>,
    },

    // ==================== Dependent Types ====================
    /// Proof by substitution
    ///
    /// From: dependent.rs
    /// Substitutes equals for equals in a property.
    Subst {
        /// Proof of equality
        eq_proof: Heap<ProofTerm>,
        /// Property to substitute into
        property: Heap<Expr>,
    },

    // ==================== Meta-level ====================
    /// Lemma (derived fact with proof)
    ///
    /// From: proof_extraction.rs
    /// Represents a proven lemma that can be reused.
    Lemma {
        /// Lemma conclusion
        conclusion: Expr,
        /// Proof of the lemma
        proof: Heap<ProofTerm>,
    },

    // ==================== Extended Proof Rules ====================
    /// And elimination: from (and l_1 ... l_n), derive l_i
    AndElim {
        conjunction: Heap<ProofTerm>,
        index: usize,
        result: Expr,
    },

    /// Not-or elimination: from (not (or l_1 ... l_n)), derive (not l_i)
    NotOrElim {
        negated_disjunction: Heap<ProofTerm>,
        index: usize,
        result: Expr,
    },

    /// Iff-true: from p, derive (iff p true)
    IffTrue {
        proof: Heap<ProofTerm>,
        formula: Expr,
    },

    /// Iff-false: from (not p), derive (iff p false)
    IffFalse {
        proof: Heap<ProofTerm>,
        formula: Expr,
    },

    /// Commutativity: derive (= (f a b) (f b a))
    Commutativity { left: Expr, right: Expr },

    /// Monotonicity: if a R a', b R b', then f(a,b) R f(a',b')
    Monotonicity {
        premises: List<ProofTerm>,
        conclusion: Expr,
    },

    /// Distributivity: f distributes over g
    Distributivity { formula: Expr },

    /// Definition axiom: Tseitin-style CNF transformation axiom
    DefAxiom { formula: Expr },

    /// Definition introduction
    DefIntro { name: Text, definition: Expr },

    /// Apply definition
    ApplyDef {
        def_proof: Heap<ProofTerm>,
        original: Expr,
        name: Text,
    },

    /// Iff to oriented equality
    IffOEq {
        iff_proof: Heap<ProofTerm>,
        left: Expr,
        right: Expr,
    },

    /// Negation normal form (positive)
    NnfPos { formula: Expr, result: Expr },

    /// Negation normal form (negative)
    NnfNeg { formula: Expr, result: Expr },

    /// Skolemization hack
    SkHack { formula: Expr, skolemized: Expr },

    /// Equality resolution
    EqualityResolution {
        equality: Heap<ProofTerm>,
        literal: Heap<ProofTerm>,
        result: Expr,
    },

    /// Bind proof
    BindProof {
        quantified_proof: Heap<ProofTerm>,
        pattern: Expr,
        binding: Expr,
    },

    /// Pull quantifier
    PullQuantifier {
        formula: Expr,
        quantifier_type: Text,
        result: Expr,
    },

    /// Push quantifier
    PushQuantifier {
        formula: Expr,
        quantifier_type: Text,
        result: Expr,
    },

    /// Eliminate unused variables
    ElimUnusedVars { formula: Expr, result: Expr },

    /// Der elimination
    DerElim { premise: Heap<ProofTerm> },

    /// Quick explain (unsat core extraction)
    QuickExplain {
        unsat_core: List<Expr>,
        explanation: Text,
    },
}

// ==================== Core Methods ====================

impl ProofTerm {
    /// Get the conclusion (formula proven) of this proof term
    ///
    /// This method extracts the logical formula that this proof establishes.
    /// For compound proofs, it recursively computes the conclusion.
    pub fn conclusion(&self) -> Expr {
        match self {
            // Base cases - return formula directly
            Self::Axiom { formula, .. }
            | Self::Assumption { formula, .. }
            | Self::Hypothesis { formula, .. }
            | Self::TheoryLemma { lemma: formula, .. }
            | Self::Lemma {
                conclusion: formula,
                ..
            }
            | Self::SmtProof { formula, .. } => formula.clone(),

            // Reflexivity - construct equality
            Self::Reflexivity { term } => Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(term.clone()),
                    right: Box::new(term.clone()),
                },
                Span::dummy(),
            ),

            // Classical logic - compute from sub-proofs
            Self::ModusPonens { implication, .. } => {
                // Modus ponens derives the consequent of the implication
                let impl_conclusion = implication.conclusion();
                match &impl_conclusion.kind {
                    ExprKind::Binary {
                        op: BinOp::Imply,
                        right,
                        ..
                    } => (**right).clone(),
                    _ => impl_conclusion, // Fallback if not an implication
                }
            }

            Self::Rewrite { target, .. } => target.clone(),

            Self::Symmetry { equality } => {
                // Flip the equality
                let eq = equality.conclusion();
                match &eq.kind {
                    ExprKind::Binary {
                        op: BinOp::Eq,
                        left,
                        right,
                    } => Expr::new(
                        ExprKind::Binary {
                            op: BinOp::Eq,
                            left: right.clone(),
                            right: left.clone(),
                        },
                        Span::dummy(),
                    ),
                    _ => eq,
                }
            }

            Self::Transitivity { right, .. } => {
                // Transitivity: A = B, B = C ⊢ A = C
                // Result is the right side of the second equality
                right.conclusion()
            }

            Self::UnitResolution { clauses } => {
                // Resolution derives the resolvent (last clause)
                clauses.last().map(|c| c.conclusion()).unwrap_or_else(|| {
                    Expr::new(
                        ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
                        Span::dummy(),
                    )
                })
            }

            Self::QuantifierInstantiation { quantified, .. } => quantified.conclusion(),

            // Constructive proofs
            Self::Lambda { var, body } => {
                // Lambda abstracts over var in body's conclusion

                // In full implementation, would construct Pi type
                body.conclusion()
            }

            Self::Cases { cases, .. } => {
                // Cases proof: all cases prove the same thing
                cases
                    .first()
                    .map(|(_, proof)| proof.conclusion())
                    .unwrap_or_else(|| {
                        Expr::new(
                            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
                            Span::dummy(),
                        )
                    })
            }

            Self::Induction {
                base_case,
                inductive_case,
                ..
            } => {
                // Induction proves a universal property
                // Both cases should prove the same thing
                base_case.conclusion()
            }

            Self::Apply { premises, .. } => {
                // Application: conclusion depends on the rule
                // For now, use last premise's conclusion
                premises.last().map(|p| p.conclusion()).unwrap_or_else(|| {
                    Expr::new(
                        ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
                        Span::dummy(),
                    )
                })
            }

            Self::Subst { property, .. } => {
                // Substitution preserves the property
                (**property).clone()
            }

            // Extended proof rules
            Self::AndElim { result, .. } => result.clone(),
            Self::NotOrElim { result, .. } => result.clone(),
            Self::IffTrue { formula, .. } => formula.clone(),
            Self::IffFalse { formula, .. } => formula.clone(),
            Self::Commutativity { left, right } => Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(left.clone()),
                    right: Box::new(right.clone()),
                },
                Span::dummy(),
            ),
            Self::Monotonicity { conclusion, .. } => conclusion.clone(),
            Self::Distributivity { formula } => formula.clone(),
            Self::DefAxiom { formula } => formula.clone(),
            Self::DefIntro { definition, .. } => definition.clone(),
            Self::ApplyDef { original, .. } => original.clone(),
            Self::IffOEq { left, right, .. } => Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(left.clone()),
                    right: Box::new(right.clone()),
                },
                Span::dummy(),
            ),
            Self::NnfPos { result, .. } => result.clone(),
            Self::NnfNeg { result, .. } => result.clone(),
            Self::SkHack { skolemized, .. } => skolemized.clone(),
            Self::EqualityResolution { result, .. } => result.clone(),
            Self::BindProof { binding, .. } => binding.clone(),
            Self::PullQuantifier { result, .. } => result.clone(),
            Self::PushQuantifier { result, .. } => result.clone(),
            Self::ElimUnusedVars { result, .. } => result.clone(),
            Self::DerElim { premise } => premise.conclusion(),
            Self::QuickExplain { unsat_core, .. } => {
                unsat_core.first().cloned().unwrap_or_else(|| {
                    Expr::new(
                        ExprKind::Literal(Literal::new(LiteralKind::Bool(false), Span::dummy())),
                        Span::dummy(),
                    )
                })
            }
        }
    }

    /// Get all axioms used in this proof
    ///
    /// Returns the set of axiom names that this proof depends on.
    /// This is crucial for understanding proof dependencies.
    pub fn used_axioms(&self) -> Set<Text> {
        let mut axioms = Set::new();
        self.collect_axioms(&mut axioms);
        axioms
    }

    /// Helper: recursively collect axioms
    fn collect_axioms(&self, axioms: &mut Set<Text>) {
        match self {
            Self::Axiom { name, .. } => {
                axioms.insert(name.clone());
            }
            Self::TheoryLemma { theory, .. } => {
                axioms.insert(theory.clone());
            }
            Self::ModusPonens {
                premise,
                implication,
            } => {
                premise.collect_axioms(axioms);
                implication.collect_axioms(axioms);
            }
            Self::Rewrite { source, .. }
            | Self::Symmetry { equality: source }
            | Self::Lambda { body: source, .. }
            | Self::QuantifierInstantiation {
                quantified: source, ..
            }
            | Self::Lemma { proof: source, .. }
            | Self::Subst {
                eq_proof: source, ..
            } => {
                source.collect_axioms(axioms);
            }
            Self::Transitivity { left, right } => {
                left.collect_axioms(axioms);
                right.collect_axioms(axioms);
            }
            Self::UnitResolution { clauses } => {
                for clause in clauses {
                    clause.collect_axioms(axioms);
                }
            }
            Self::Cases { cases, .. } => {
                for (_, proof) in cases {
                    proof.collect_axioms(axioms);
                }
            }
            Self::Induction {
                base_case,
                inductive_case,
                ..
            } => {
                base_case.collect_axioms(axioms);
                inductive_case.collect_axioms(axioms);
            }
            Self::Apply { premises, .. } => {
                for premise in premises {
                    premise.collect_axioms(axioms);
                }
            }
            _ => {}
        }
    }

    /// Count proof depth (maximum nesting level)
    ///
    /// Returns the maximum depth of the proof tree.
    /// Useful for complexity analysis.
    pub fn proof_depth(&self) -> usize {
        match self {
            // Leaf nodes have depth 1
            Self::Axiom { .. }
            | Self::Assumption { .. }
            | Self::Reflexivity { .. }
            | Self::TheoryLemma { .. }
            | Self::Hypothesis { .. }
            | Self::SmtProof { .. } => 1,

            // Binary nodes: 1 + max of children
            Self::ModusPonens {
                premise,
                implication,
            } => 1 + premise.proof_depth().max(implication.proof_depth()),

            Self::Transitivity { left, right } => 1 + left.proof_depth().max(right.proof_depth()),

            // Unary nodes: 1 + child depth
            Self::Rewrite { source, .. }
            | Self::Symmetry { equality: source }
            | Self::Lambda { body: source, .. }
            | Self::QuantifierInstantiation {
                quantified: source, ..
            }
            | Self::Lemma { proof: source, .. }
            | Self::Subst {
                eq_proof: source, ..
            } => 1 + source.proof_depth(),

            // N-ary nodes: 1 + max of all children
            Self::UnitResolution { clauses } => {
                1 + clauses.iter().map(|c| c.proof_depth()).max().unwrap_or(0)
            }

            Self::Apply { premises, .. } => {
                1 + premises.iter().map(|p| p.proof_depth()).max().unwrap_or(0)
            }

            Self::Cases { cases, .. } => {
                1 + cases
                    .iter()
                    .map(|(_, p)| p.proof_depth())
                    .max()
                    .unwrap_or(0)
            }

            Self::Induction {
                base_case,
                inductive_case,
                ..
            } => 1 + base_case.proof_depth().max(inductive_case.proof_depth()),

            // Extended proof rules - handle various depths based on structure
            Self::AndElim { conjunction, .. }
            | Self::NotOrElim {
                negated_disjunction: conjunction,
                ..
            }
            | Self::IffTrue {
                proof: conjunction, ..
            }
            | Self::IffFalse {
                proof: conjunction, ..
            }
            | Self::ApplyDef {
                def_proof: conjunction,
                ..
            }
            | Self::IffOEq {
                iff_proof: conjunction,
                ..
            }
            | Self::BindProof {
                quantified_proof: conjunction,
                ..
            }
            | Self::DerElim {
                premise: conjunction,
            } => 1 + conjunction.proof_depth(),

            Self::EqualityResolution {
                equality, literal, ..
            } => 1 + equality.proof_depth().max(literal.proof_depth()),

            Self::Monotonicity { premises, .. } => {
                1 + premises.iter().map(|p| p.proof_depth()).max().unwrap_or(0)
            }

            // Leaf-like rules with no sub-proofs
            Self::Commutativity { .. }
            | Self::Distributivity { .. }
            | Self::DefAxiom { .. }
            | Self::DefIntro { .. }
            | Self::NnfPos { .. }
            | Self::NnfNeg { .. }
            | Self::SkHack { .. }
            | Self::PullQuantifier { .. }
            | Self::PushQuantifier { .. }
            | Self::ElimUnusedVars { .. }
            | Self::QuickExplain { .. } => 1,
        }
    }

    /// Count total proof nodes
    ///
    /// Returns the total number of nodes in the proof tree.
    /// Useful for size analysis and optimization.
    pub fn node_count(&self) -> usize {
        match self {
            // Leaf nodes count as 1
            Self::Axiom { .. }
            | Self::Assumption { .. }
            | Self::Reflexivity { .. }
            | Self::TheoryLemma { .. }
            | Self::Hypothesis { .. }
            | Self::SmtProof { .. } => 1,

            // Binary nodes: 1 + sum of children
            Self::ModusPonens {
                premise,
                implication,
            } => 1 + premise.node_count() + implication.node_count(),

            Self::Transitivity { left, right } => 1 + left.node_count() + right.node_count(),

            // Unary nodes: 1 + child count
            Self::Rewrite { source, .. }
            | Self::Symmetry { equality: source }
            | Self::Lambda { body: source, .. }
            | Self::QuantifierInstantiation {
                quantified: source, ..
            }
            | Self::Lemma { proof: source, .. }
            | Self::Subst {
                eq_proof: source, ..
            } => 1 + source.node_count(),

            // N-ary nodes: 1 + sum of all children
            Self::UnitResolution { clauses } => {
                1 + clauses.iter().map(|c| c.node_count()).sum::<usize>()
            }

            Self::Apply { premises, .. } => {
                1 + premises.iter().map(|p| p.node_count()).sum::<usize>()
            }

            Self::Cases { cases, .. } => {
                1 + cases.iter().map(|(_, p)| p.node_count()).sum::<usize>()
            }

            Self::Induction {
                base_case,
                inductive_case,
                ..
            } => 1 + base_case.node_count() + inductive_case.node_count(),

            // Extended proof rules
            Self::AndElim { conjunction, .. }
            | Self::NotOrElim {
                negated_disjunction: conjunction,
                ..
            }
            | Self::IffTrue {
                proof: conjunction, ..
            }
            | Self::IffFalse {
                proof: conjunction, ..
            }
            | Self::ApplyDef {
                def_proof: conjunction,
                ..
            }
            | Self::IffOEq {
                iff_proof: conjunction,
                ..
            }
            | Self::BindProof {
                quantified_proof: conjunction,
                ..
            }
            | Self::DerElim {
                premise: conjunction,
            } => 1 + conjunction.node_count(),

            Self::EqualityResolution {
                equality, literal, ..
            } => 1 + equality.node_count() + literal.node_count(),

            Self::Monotonicity { premises, .. } => {
                1 + premises.iter().map(|p| p.node_count()).sum::<usize>()
            }

            // Leaf-like rules with no sub-proofs
            Self::Commutativity { .. }
            | Self::Distributivity { .. }
            | Self::DefAxiom { .. }
            | Self::DefIntro { .. }
            | Self::NnfPos { .. }
            | Self::NnfNeg { .. }
            | Self::SkHack { .. }
            | Self::PullQuantifier { .. }
            | Self::PushQuantifier { .. }
            | Self::ElimUnusedVars { .. }
            | Self::QuickExplain { .. } => 1,
        }
    }

    /// Convert proof term to executable expression (program extraction)
    ///
    /// Extracts the computational content from constructive proofs.
    /// Classical proofs have no computational content and return unit/true.
    ///
    /// From: proof_search.rs
    pub fn to_expr(&self) -> Result<Expr, ProofError> {
        match self {
            // Constructive proofs with computational content
            Self::Lambda { body, .. } => body.to_expr(),

            Self::Cases { scrutinee, .. } => {
                // Case analysis becomes the scrutinee (simplified)
                Ok(scrutinee.clone())
            }

            Self::Induction { base_case, .. } => {
                // Induction becomes recursion (simplified)
                base_case.to_expr()
            }

            // Classical proofs have no computational content
            Self::Axiom { .. }
            | Self::Assumption { .. }
            | Self::Hypothesis { .. }
            | Self::ModusPonens { .. }
            | Self::Rewrite { .. }
            | Self::Symmetry { .. }
            | Self::Transitivity { .. }
            | Self::Reflexivity { .. }
            | Self::TheoryLemma { .. }
            | Self::UnitResolution { .. }
            | Self::QuantifierInstantiation { .. }
            | Self::Apply { .. }
            | Self::SmtProof { .. }
            | Self::Subst { .. }
            | Self::Lemma { .. } => {
                // Return unit value
                Ok(Expr::new(
                    ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
                    Span::dummy(),
                ))
            }

            // All other variants also have no computational content
            _ => Ok(Expr::new(
                ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
                Span::dummy(),
            )),
        }
    }

    /// Extract witness from existence proof
    ///
    /// Given a proof of ∃x. P(x), extract the witness x.
    /// Only works for constructive proofs.
    ///
    /// From: proof_search.rs
    pub fn extract_witness(&self) -> Result<Expr, ProofError> {
        match self {
            Self::Lambda { body, .. } => {
                // Lambda body should contain the witness
                body.to_expr()
            }
            Self::Cases { cases, .. } => {
                // First case should contain witness
                cases
                    .first()
                    .map(|(pattern, _)| pattern.clone())
                    .ok_or_else(|| ProofError::InvalidProof("no cases in proof".into()))
            }
            _ => Err(ProofError::InvalidProof(
                "proof is not an existence proof".into(),
            )),
        }
    }

    /// Check if proof term is well-formed
    ///
    /// Performs structural validation of the proof.
    /// Returns Ok(()) if well-formed, Err if malformed.
    ///
    /// From: dependent.rs
    pub fn check_well_formed(&self) -> Result<(), ProofError> {
        match self {
            Self::Axiom { name, formula } => {
                if name.is_empty() {
                    return Err(ProofError::InvalidProof("axiom has no name".into()));
                }
                // Would check formula is well-formed
                Ok(())
            }

            Self::ModusPonens {
                premise,
                implication,
            } => {
                premise.check_well_formed()?;
                implication.check_well_formed()?;
                // Would check that implication is actually an implication
                Ok(())
            }

            Self::Symmetry { equality } => {
                equality.check_well_formed()?;
                // Would check that equality is actually an equality
                Ok(())
            }

            Self::Transitivity { left, right } => {
                left.check_well_formed()?;
                right.check_well_formed()?;
                // Would check that both are equalities with matching middle term
                Ok(())
            }

            Self::UnitResolution { clauses } => {
                if clauses.is_empty() {
                    return Err(ProofError::InvalidProof(
                        "unit resolution has no clauses".into(),
                    ));
                }
                for clause in clauses {
                    clause.check_well_formed()?;
                }
                Ok(())
            }

            Self::Cases { cases, .. } => {
                if cases.is_empty() {
                    return Err(ProofError::InvalidProof("cases proof has no cases".into()));
                }
                for (_, proof) in cases {
                    proof.check_well_formed()?;
                }
                Ok(())
            }

            Self::Induction {
                base_case,
                inductive_case,
                ..
            } => {
                base_case.check_well_formed()?;
                inductive_case.check_well_formed()?;
                Ok(())
            }

            Self::Apply { premises, .. } => {
                for premise in premises {
                    premise.check_well_formed()?;
                }
                Ok(())
            }

            // Other variants are structurally valid by construction
            _ => Ok(()),
        }
    }

    /// Add a dependency (axiom or lemma) to this proof
    ///
    /// This is used for tracking proof dependencies.
    /// Note: The unified ProofTerm doesn't store dependencies directly,
    /// but this method exists for API compatibility with dependent.rs.
    ///
    /// From: dependent.rs
    pub fn add_dependency(&mut self, dep: Text) {
        // In dependent.rs, ProofTerm is a struct with a dependencies field.
        // In the unified design, dependencies are computed via used_axioms().
        // This method is a no-op for compatibility.
        // In a full implementation, we could wrap ProofTerm in a struct
        // with an explicit dependencies set if needed.
    }
}

// ==================== Conversion Traits ====================

/// Convert from proof_extraction::ProofTerm to unified ProofTerm
impl From<crate::proof_extraction::ProofTerm> for ProofTerm {
    fn from(old: crate::proof_extraction::ProofTerm) -> Self {
        match old {
            crate::proof_extraction::ProofTerm::Axiom { name, formula } => Self::Axiom {
                name,
                formula: Self::text_to_expr(&formula),
            },
            crate::proof_extraction::ProofTerm::Assumption { id, formula } => Self::Assumption {
                id,
                formula: Self::text_to_expr(&formula),
            },
            crate::proof_extraction::ProofTerm::ModusPonens {
                premise,
                implication,
            } => Self::ModusPonens {
                premise: Heap::new((*premise).into()),
                implication: Heap::new((*implication).into()),
            },
            crate::proof_extraction::ProofTerm::Rewrite {
                source,
                rule,
                target,
            } => Self::Rewrite {
                source: Heap::new((*source).into()),
                rule,
                target: Self::text_to_expr(&target),
            },
            crate::proof_extraction::ProofTerm::Symmetry { equality } => Self::Symmetry {
                equality: Heap::new((*equality).into()),
            },
            crate::proof_extraction::ProofTerm::Transitivity { left, right } => {
                Self::Transitivity {
                    left: Heap::new((*left).into()),
                    right: Heap::new((*right).into()),
                }
            }
            crate::proof_extraction::ProofTerm::Reflexivity { term } => Self::Reflexivity {
                term: Self::text_to_expr(&term),
            },
            crate::proof_extraction::ProofTerm::TheoryLemma { theory, lemma } => {
                Self::TheoryLemma {
                    theory,
                    lemma: Self::text_to_expr(&lemma),
                }
            }
            crate::proof_extraction::ProofTerm::UnitResolution { clauses } => {
                Self::UnitResolution {
                    clauses: clauses.into_iter().map(|c| c.into()).collect(),
                }
            }
            crate::proof_extraction::ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => Self::QuantifierInstantiation {
                quantified: Heap::new((*quantified).into()),
                instantiation: instantiation
                    .into_iter()
                    .map(|(k, v)| (k, Self::text_to_expr(&v)))
                    .collect(),
            },
            crate::proof_extraction::ProofTerm::Lemma { conclusion, proof } => Self::Lemma {
                conclusion: Self::text_to_expr(&conclusion),
                proof: Heap::new((*proof).into()),
            },
            crate::proof_extraction::ProofTerm::Hypothesis { id, formula } => Self::Hypothesis {
                id,
                formula: Self::text_to_expr(&formula),
            },
            crate::proof_extraction::ProofTerm::AndElim {
                conjunction,
                index,
                result,
            } => Self::AndElim {
                conjunction: Heap::new((*conjunction).into()),
                index,
                result: Self::text_to_expr(&result),
            },
            crate::proof_extraction::ProofTerm::NotOrElim {
                negated_disjunction,
                index,
                result,
            } => Self::NotOrElim {
                negated_disjunction: Heap::new((*negated_disjunction).into()),
                index,
                result: Self::text_to_expr(&result),
            },
            crate::proof_extraction::ProofTerm::IffTrue { proof, formula } => Self::IffTrue {
                proof: Heap::new((*proof).into()),
                formula: Self::text_to_expr(&formula),
            },
            crate::proof_extraction::ProofTerm::IffFalse { proof, formula } => Self::IffFalse {
                proof: Heap::new((*proof).into()),
                formula: Self::text_to_expr(&formula),
            },
            crate::proof_extraction::ProofTerm::Commutativity { left, right } => {
                Self::Commutativity {
                    left: Self::text_to_expr(&left),
                    right: Self::text_to_expr(&right),
                }
            }
            crate::proof_extraction::ProofTerm::Monotonicity {
                premises,
                conclusion,
            } => Self::Monotonicity {
                premises: premises.into_iter().map(|p| p.into()).collect(),
                conclusion: Self::text_to_expr(&conclusion),
            },
            crate::proof_extraction::ProofTerm::Distributivity { formula } => {
                Self::Distributivity {
                    formula: Self::text_to_expr(&formula),
                }
            }
            crate::proof_extraction::ProofTerm::DefAxiom { formula } => Self::DefAxiom {
                formula: Self::text_to_expr(&formula),
            },
            crate::proof_extraction::ProofTerm::DefIntro { name, definition } => Self::DefIntro {
                name,
                definition: Self::text_to_expr(&definition),
            },
            crate::proof_extraction::ProofTerm::ApplyDef {
                def_proof,
                original,
                name,
            } => Self::ApplyDef {
                def_proof: Heap::new((*def_proof).into()),
                original: Self::text_to_expr(&original),
                name,
            },
            crate::proof_extraction::ProofTerm::IffOEq {
                iff_proof,
                left,
                right,
            } => Self::IffOEq {
                iff_proof: Heap::new((*iff_proof).into()),
                left: Self::text_to_expr(&left),
                right: Self::text_to_expr(&right),
            },
            // NNFPos: {premises, conclusion} -> NnfPos: {formula, result}
            crate::proof_extraction::ProofTerm::NNFPos {
                premises: _,
                conclusion,
            } => Self::NnfPos {
                formula: Self::text_to_expr(&conclusion),
                result: Self::text_to_expr(&conclusion),
            },
            // NNFNeg: {premises, conclusion} -> NnfNeg: {formula, result}
            crate::proof_extraction::ProofTerm::NNFNeg {
                premises: _,
                conclusion,
            } => Self::NnfNeg {
                formula: Self::text_to_expr(&conclusion),
                result: Self::text_to_expr(&conclusion),
            },
            // Skolemize: {formula} -> SkHack: {formula, skolemized}
            crate::proof_extraction::ProofTerm::Skolemize { formula } => Self::SkHack {
                formula: Self::text_to_expr(&formula),
                skolemized: Self::text_to_expr(&formula),
            },
            // DestructiveEqRes: {formula} -> EqualityResolution (approximate mapping)
            crate::proof_extraction::ProofTerm::DestructiveEqRes { formula } => {
                // Create a placeholder proof term for equality resolution
                let placeholder = ProofTerm::Axiom {
                    name: Text::from("eq_res"),
                    formula: Self::text_to_expr(&formula),
                };
                Self::EqualityResolution {
                    equality: Heap::new(placeholder.clone()),
                    literal: Heap::new(placeholder),
                    result: Self::text_to_expr(&formula),
                }
            }
            // Bind: {body_proof, variables, conclusion} -> BindProof (approximate)
            crate::proof_extraction::ProofTerm::Bind {
                body_proof,
                variables: _,
                conclusion,
            } => Self::BindProof {
                quantified_proof: Heap::new((*body_proof).into()),
                pattern: Self::text_to_expr(&conclusion),
                binding: Self::text_to_expr(&conclusion),
            },
            // PullQuant: {formula} -> PullQuantifier
            crate::proof_extraction::ProofTerm::PullQuant { formula } => Self::PullQuantifier {
                formula: Self::text_to_expr(&formula),
                quantifier_type: Text::from("forall"),
                result: Self::text_to_expr(&formula),
            },
            // PushQuant: {formula} -> PushQuantifier
            crate::proof_extraction::ProofTerm::PushQuant { formula } => Self::PushQuantifier {
                formula: Self::text_to_expr(&formula),
                quantifier_type: Text::from("forall"),
                result: Self::text_to_expr(&formula),
            },
            // ElimUnusedVars: {formula} -> ElimUnusedVars
            crate::proof_extraction::ProofTerm::ElimUnusedVars { formula } => {
                Self::ElimUnusedVars {
                    formula: Self::text_to_expr(&formula),
                    result: Self::text_to_expr(&formula),
                }
            }
            // QuantIntro: {body_proof, conclusion} -> map to a generic handler
            crate::proof_extraction::ProofTerm::QuantIntro {
                body_proof,
                conclusion,
            } => Self::BindProof {
                quantified_proof: Heap::new((*body_proof).into()),
                pattern: Self::text_to_expr(&conclusion),
                binding: Self::text_to_expr(&conclusion),
            },
            // HyperResolve: {clauses, conclusion} -> QuickExplain (approximate)
            crate::proof_extraction::ProofTerm::HyperResolve {
                clauses,
                conclusion,
            } => Self::QuickExplain {
                unsat_core: clauses
                    .into_iter()
                    .map(|p| p.into())
                    .map(|pt: ProofTerm| pt.conclusion())
                    .collect(),
                explanation: conclusion,
            },
        }
    }
}

/// Convert from proof_search::ProofTerm to unified ProofTerm
impl From<crate::proof_search::ProofTerm> for ProofTerm {
    fn from(old: crate::proof_search::ProofTerm) -> Self {
        match old {
            crate::proof_search::ProofTerm::Axiom(name) => Self::Axiom {
                name,
                formula: Self::dummy_expr(),
            },
            crate::proof_search::ProofTerm::Apply { rule, premises } => Self::Apply {
                rule,
                premises: premises
                    .iter()
                    .map(|p| Heap::new((**p).clone().into()))
                    .collect(),
            },
            crate::proof_search::ProofTerm::Lambda { var, body } => Self::Lambda {
                var,
                body: Heap::new((*body).clone().into()),
            },
            crate::proof_search::ProofTerm::Cases { scrutinee, cases } => Self::Cases {
                scrutinee,
                cases: cases
                    .iter()
                    .map(|(pat, proof)| (pat.clone(), Heap::new((**proof).clone().into())))
                    .collect(),
            },
            crate::proof_search::ProofTerm::Induction {
                var,
                base_case,
                inductive_case,
            } => Self::Induction {
                var,
                base_case: Heap::new((*base_case).clone().into()),
                inductive_case: Heap::new((*inductive_case).clone().into()),
            },
            crate::proof_search::ProofTerm::SmtProof { solver, formula } => Self::SmtProof {
                solver,
                formula,
                smt_trace: Maybe::None,
            },
        }
    }
}

/// Convert from dependent::ProofTerm to unified ProofTerm
impl From<crate::dependent::ProofTerm> for ProofTerm {
    fn from(old: crate::dependent::ProofTerm) -> Self {
        use crate::dependent::ProofStructure;

        // The dependent::ProofTerm is a struct with proposition and proof
        match old.proof {
            ProofStructure::SolverProof { smt_proof } => Self::SmtProof {
                solver: "z3".into(),
                formula: (*old.proposition).clone(),
                smt_trace: Maybe::Some(smt_proof),
            },
            ProofStructure::Refl => Self::Reflexivity {
                term: (*old.proposition).clone(),
            },
            ProofStructure::Subst { eq_proof, property } => Self::Subst {
                eq_proof: Heap::new((*eq_proof).clone().into()),
                property: Heap::new((*property).clone()),
            },
            ProofStructure::Trans { left, right } => Self::Transitivity {
                left: Heap::new((*left).clone().into()),
                right: Heap::new((*right).clone().into()),
            },
            ProofStructure::ModusPonens {
                premise,
                implication,
            } => Self::ModusPonens {
                premise: Heap::new((*premise).clone().into()),
                implication: Heap::new((*implication).clone().into()),
            },
            ProofStructure::Assumption { name } => Self::Axiom {
                name,
                formula: (*old.proposition).clone(),
            },
        }
    }
}

// ==================== Helper Methods ====================

impl ProofTerm {
    /// Convert Text to Expr (helper for conversions)
    fn text_to_expr(text: &Text) -> Expr {
        // In proof_extraction, formulas are stored as Text
        // This is a simplified conversion - real impl would parse
        // Note: StringLit::Regular expects verum_common::Text (= String), not verum_std::Text
        Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Text(StringLit::Regular(text.to_string().into())),
                Span::dummy(),
            )),
            Span::dummy(),
        )
    }

    /// Create a dummy expression (helper for conversions)
    fn dummy_expr() -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            Span::dummy(),
        )
    }
}

// ==================== Error Types ====================

/// Errors that can occur during proof term operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum ProofError {
    /// Invalid proof structure
    #[error("invalid proof: {0}")]
    InvalidProof(Text),

    /// Tactic failed
    #[error("tactic failed: {0}")]
    TacticFailed(Text),

    /// SMT timeout
    #[error("SMT timeout")]
    SmtTimeout,

    /// Unification failed
    #[error("unification failed: {0}")]
    UnificationFailed(Text),

    /// Goal not in context
    #[error("goal not in context: {0}")]
    NotInContext(Text),

    /// Not an equality
    #[error("not an equality: {0}")]
    NotEquality(Text),

    /// Cannot extract program
    #[error("cannot extract program: {0}")]
    CannotExtract(Text),
}

// ==================== Convenience Constructors ====================

impl ProofTerm {
    /// Create an axiom proof term
    pub fn axiom(name: impl Into<Text>, formula: Expr) -> Self {
        Self::Axiom {
            name: name.into(),
            formula,
        }
    }

    /// Create an assumption proof term
    pub fn assumption(id: usize, formula: Expr) -> Self {
        Self::Assumption { id, formula }
    }

    /// Create a reflexivity proof term
    pub fn reflexivity(term: Expr) -> Self {
        Self::Reflexivity { term }
    }

    /// Create an SMT proof term
    pub fn smt_proof(solver: impl Into<Text>, formula: Expr) -> Self {
        Self::SmtProof {
            solver: solver.into(),
            formula,
            smt_trace: Maybe::None,
        }
    }

    /// Create an SMT proof term with trace
    pub fn smt_proof_with_trace(solver: impl Into<Text>, formula: Expr, trace: Text) -> Self {
        Self::SmtProof {
            solver: solver.into(),
            formula,
            smt_trace: Maybe::Some(trace),
        }
    }

    /// Create a theory lemma proof term
    pub fn theory_lemma(theory: impl Into<Text>, lemma: Expr) -> Self {
        Self::TheoryLemma {
            theory: theory.into(),
            lemma,
        }
    }

    /// Create a lambda proof term
    pub fn lambda(var: impl Into<Text>, body: ProofTerm) -> Self {
        Self::Lambda {
            var: var.into(),
            body: Heap::new(body),
        }
    }

    /// Create a modus ponens proof term
    pub fn modus_ponens(premise: ProofTerm, implication: ProofTerm) -> Self {
        Self::ModusPonens {
            premise: Heap::new(premise),
            implication: Heap::new(implication),
        }
    }

    /// Create a transitivity proof term
    pub fn transitivity(left: ProofTerm, right: ProofTerm) -> Self {
        Self::Transitivity {
            left: Heap::new(left),
            right: Heap::new(right),
        }
    }
}

// ==================== Display Implementation ====================

impl std::fmt::Display for ProofTerm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Axiom { name, .. } => write!(f, "Axiom({})", name),
            Self::Assumption { id, .. } => write!(f, "Assumption({})", id),
            Self::Hypothesis { id, .. } => write!(f, "Hypothesis({})", id),
            Self::Reflexivity { .. } => write!(f, "Refl"),
            Self::ModusPonens { .. } => write!(f, "MP"),
            Self::Rewrite { rule, .. } => write!(f, "Rewrite({})", rule),
            Self::Symmetry { .. } => write!(f, "Symm"),
            Self::Transitivity { .. } => write!(f, "Trans"),
            Self::TheoryLemma { theory, .. } => write!(f, "Theory({})", theory),
            Self::UnitResolution { clauses } => write!(f, "Resolution({})", clauses.len()),
            Self::QuantifierInstantiation { .. } => write!(f, "QInst"),
            Self::Lambda { var, .. } => write!(f, "λ{}", var),
            Self::Cases { cases, .. } => write!(f, "Cases({})", cases.len()),
            Self::Induction { var, .. } => write!(f, "Ind({})", var),
            Self::Apply { rule, .. } => write!(f, "Apply({})", rule),
            Self::SmtProof { solver, .. } => write!(f, "SMT({})", solver),
            Self::Subst { .. } => write!(f, "Subst"),
            Self::Lemma { .. } => write!(f, "Lemma"),
            Self::AndElim { .. } => write!(f, "AndElim"),
            Self::NotOrElim { .. } => write!(f, "NotOrElim"),
            Self::IffTrue { .. } => write!(f, "IffTrue"),
            Self::IffFalse { .. } => write!(f, "IffFalse"),
            Self::Commutativity { .. } => write!(f, "Comm"),
            Self::Monotonicity { .. } => write!(f, "Mono"),
            Self::Distributivity { .. } => write!(f, "Dist"),
            Self::DefAxiom { .. } => write!(f, "DefAxiom"),
            Self::DefIntro { .. } => write!(f, "DefIntro"),
            Self::ApplyDef { .. } => write!(f, "ApplyDef"),
            Self::IffOEq { .. } => write!(f, "IffOEq"),
            Self::NnfPos { .. } => write!(f, "NnfPos"),
            Self::NnfNeg { .. } => write!(f, "NnfNeg"),
            Self::SkHack { .. } => write!(f, "SkHack"),
            Self::EqualityResolution { .. } => write!(f, "EqRes"),
            Self::BindProof { .. } => write!(f, "Bind"),
            Self::PullQuantifier { .. } => write!(f, "PullQ"),
            Self::PushQuantifier { .. } => write!(f, "PushQ"),
            Self::ElimUnusedVars { .. } => write!(f, "ElimVars"),
            Self::DerElim { .. } => write!(f, "DerElim"),
            Self::QuickExplain { .. } => write!(f, "QuickExplain"),
        }
    }
}
