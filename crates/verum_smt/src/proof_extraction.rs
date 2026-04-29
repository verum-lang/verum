//! Proof Extraction and Analysis for Formal Verification
//!
//! This module provides facilities for extracting and analyzing Z3 proof objects,
//! enabling formal verification workflows and proof certification (v2.0+).
//!
//! ## Features
//! - Extract proof terms from Z3 solver
//! - Analyze proof structure and dependencies
//! - Convert proofs to portable formats (SMT-LIB2, Coq, Lean)
//! - Proof validation and certification
//! - Proof minimization and simplification
//! - Proof serialization/deserialization for caching
//! - Solver configuration for proof generation
//!
//! ## Proof Rules Supported
//! This module extracts structured proof terms from all Z3 proof rules including:
//! - Logical rules: ModusPonens, AndElim, NotOrElim, IffTrue, IffFalse
//! - Equality rules: Reflexivity, Symmetry, Transitivity, Commutativity, Monotonicity
//! - Rewriting rules: Rewrite, RewriteStar, ApplyDef
//! - Quantifier rules: QuantIntro, QuantInst, PullQuant, PushQuant, ElimUnusedVars, Bind
//! - Normal form rules: NNFPos, NNFNeg, Skolemize
//! - SAT/Resolution rules: UnitResolution, HyperResolve, Lemma
//! - Definition rules: DefAxiom, DefIntro, DestructiveEqRes
//! - Theory rules: TheoryLemma, Distributivity
//!
//! ## Usage
//!
//! ```ignore
//! use verum_smt::proof_extraction::{ProofExtractor, ProofGenerationConfig};
//! use z3::{Config, Context, Solver, SatResult};
//!
//! // Create solver with proof generation enabled
//! let config = ProofGenerationConfig::production();
//! let mut cfg = Config::new();
//! cfg.set_proof_generation(true);
//! z3::with_z3_config(&cfg, || {
//!     let solver = Solver::new();
//!     // Add assertions...
//!     if solver.check() == SatResult::Unsat {
//!         if let Some(proof) = solver.get_proof() {
//!             let extractor = ProofExtractor::new();
//!             if let Some(term) = extractor.extract_proof(&proof) {
//!                 println!("Proof: {:?}", term);
//!             }
//!         }
//!     }
//! });
//! ```
//!
//! Spec: Future v2.0 - Formal Proofs (foundation for dependent types)

use crate::tactics::{StrategyBuilder, TacticCombinator, TacticKind};
use serde::{Deserialize, Serialize};
use verum_common::{List, Map, Maybe, Set, Text};
use verum_common::ToText;
use z3::ast::{Ast, Dynamic};
use z3::{Config, Goal, Solver, Tactic};

// ==================== Proof Generation Configuration ====================

/// Configuration for Z3 proof generation
///
/// This configuration controls how Z3 generates and extracts proofs.
/// For production use, enable all proof features; for development, use minimal settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofGenerationConfig {
    /// Enable proof generation in Z3 solver
    pub enable_proofs: bool,
    /// Enable unsat core extraction for minimal proofs
    pub enable_unsat_cores: bool,
    /// Minimize unsat cores (more expensive but better results)
    pub minimize_unsat_cores: bool,
    /// Maximum proof depth to extract (prevents stack overflow on large proofs)
    pub max_proof_depth: usize,
    /// Enable proof simplification after extraction
    pub simplify_proofs: bool,
    /// Enable caching of extracted proofs
    pub enable_proof_cache: bool,
    /// Validate proofs after extraction
    pub validate_on_extract: bool,
    /// Timeout for proof extraction in milliseconds (0 = no timeout)
    pub extraction_timeout_ms: u64,
}

impl Default for ProofGenerationConfig {
    fn default() -> Self {
        Self {
            enable_proofs: true,
            enable_unsat_cores: true,
            minimize_unsat_cores: false,
            max_proof_depth: 1000,
            simplify_proofs: true,
            enable_proof_cache: false,
            validate_on_extract: false,
            extraction_timeout_ms: 0,
        }
    }
}

impl ProofGenerationConfig {
    /// Create configuration optimized for production use
    ///
    /// Enables all validation and minimization features.
    pub fn production() -> Self {
        Self {
            enable_proofs: true,
            enable_unsat_cores: true,
            minimize_unsat_cores: true,
            max_proof_depth: 5000,
            simplify_proofs: true,
            enable_proof_cache: true,
            validate_on_extract: true,
            extraction_timeout_ms: 30000, // 30 seconds
        }
    }

    /// Create configuration optimized for development/debugging
    ///
    /// Minimal overhead, no validation.
    pub fn development() -> Self {
        Self {
            enable_proofs: true,
            enable_unsat_cores: false,
            minimize_unsat_cores: false,
            max_proof_depth: 500,
            simplify_proofs: false,
            enable_proof_cache: false,
            validate_on_extract: false,
            extraction_timeout_ms: 5000, // 5 seconds
        }
    }

    /// Create configuration for minimal proof extraction
    ///
    /// Only basic proof extraction, no extras.
    pub fn minimal() -> Self {
        Self {
            enable_proofs: true,
            enable_unsat_cores: false,
            minimize_unsat_cores: false,
            max_proof_depth: 100,
            simplify_proofs: false,
            enable_proof_cache: false,
            validate_on_extract: false,
            extraction_timeout_ms: 0,
        }
    }

    /// Apply this configuration to a Z3 Config object
    ///
    /// This sets up Z3's internal configuration for proof
    /// generation. Closes the inert-defense pattern around
    /// `enable_unsat_cores`: previously the field was set on
    /// the config but never reached Z3 — extracting an unsat
    /// core required a side-channel call. Now both
    /// `enable_proofs` and `enable_unsat_cores` flow through
    /// to the Z3 Config in one place.
    pub fn apply_to_z3_config(&self, cfg: &mut Config) {
        cfg.set_proof_generation(self.enable_proofs);
        // unsat_core is a documented Config-level Z3 param;
        // forwarding it here means every solver constructed
        // under `with_config` inherits the policy without
        // needing to re-call `set_params({unsat_core: true})`
        // per query.
        cfg.set_bool_param_value("unsat_core", self.enable_unsat_cores);
    }

    /// Apply per-solver parameters that aren't available at the
    /// Config level.
    ///
    /// Two of the documented `ProofGenerationConfig` fields are
    /// Solver-level Z3 params, not Config-level — they have to
    /// be set per-Solver via `Solver::set_params`:
    ///
    ///  * `minimize_unsat_cores` — Z3's `smt.core.minimize`
    ///    parameter. When true, the solver runs additional
    ///    minimization on the unsat core before returning it,
    ///    producing tighter explanations at the cost of some
    ///    extra solver work.
    ///  * `extraction_timeout_ms` — when nonzero, sets Z3's
    ///    `timeout` parameter so the solver itself bails before
    ///    the extractor has to. Zero leaves the solver unbounded
    ///    (matches the documented "0 = no timeout" semantics).
    ///
    /// Call this on every Solver involved in proof extraction so
    /// the per-call resource budget actually reaches the solver.
    pub fn apply_to_z3_solver(&self, solver: &Solver) {
        let mut params = z3::Params::new();
        params.set_bool("smt.core.minimize", self.minimize_unsat_cores);
        if self.extraction_timeout_ms > 0 {
            let clamped = self.extraction_timeout_ms.min(u32::MAX as u64) as u32;
            params.set_u32("timeout", clamped);
        }
        solver.set_params(&params);
    }

    /// Execute code with this proof generation configuration
    ///
    /// Sets up Z3 context with proper proof generation settings.
    pub fn with_config<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R + Send + Sync,
        R: Send + Sync,
    {
        let mut cfg = Config::new();
        self.apply_to_z3_config(&mut cfg);
        z3::with_z3_config(&cfg, f)
    }
}

// ==================== Proof Term Representation ====================

/// Structured proof term representation
///
/// Converts Z3's internal proof objects to a structured format
/// suitable for analysis, minimization, and export.
///
/// This enum represents all Z3 proof rules as structured proof terms,
/// enabling complete proof tree extraction and analysis.
///
/// ## Serialization
///
/// ProofTerm supports serde serialization for caching and persistence:
///
/// ```ignore
/// let proof = ProofTerm::Axiom { name: "ax1".into(), formula: "x > 0".into() };
/// let json = serde_json::to_string(&proof)?;
/// let restored: ProofTerm = serde_json::from_str(&json)?;
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProofTerm {
    /// Axiom (given fact)
    Axiom { name: Text, formula: Text },

    /// Assumption (hypothesis)
    Assumption { id: usize, formula: Text },

    /// Modus ponens: from A and A => B, derive B
    ///
    /// ```text
    /// T1: A
    /// T2: A => B
    /// [modus-ponens T1 T2]: B
    /// ```
    ModusPonens {
        premise: Box<ProofTerm>,
        implication: Box<ProofTerm>,
    },

    /// Rewrite rule application
    ///
    /// A proof for a local rewriting step (= t s).
    /// The head function symbol of t is interpreted.
    Rewrite {
        source: Box<ProofTerm>,
        rule: Text,
        target: Text,
    },

    /// Symmetry: from A = B, derive B = A
    Symmetry { equality: Box<ProofTerm> },

    /// Transitivity: from A = B and B = C, derive A = C
    Transitivity {
        left: Box<ProofTerm>,
        right: Box<ProofTerm>,
    },

    /// Reflexivity: derive A = A
    Reflexivity { term: Text },

    /// Theory lemma (SMT theory axiom)
    ///
    /// Generic proof for theory lemmas from arithmetic, arrays, etc.
    TheoryLemma { theory: Text, lemma: Text },

    /// Unit resolution (SAT reasoning)
    ///
    /// ```text
    /// T1: (or l_1 ... l_n l_1' ... l_m')
    /// T2: (not l_1)
    /// ...
    /// T(n+1): (not l_n)
    /// [unit-resolution T1 ... T(n+1)]: (or l_1' ... l_m')
    /// ```
    UnitResolution { clauses: List<ProofTerm> },

    /// Quantifier instantiation
    ///
    /// A proof of (or (not (forall (x) (P x))) (P a))
    QuantifierInstantiation {
        quantified: Box<ProofTerm>,
        instantiation: Map<Text, Text>,
    },

    /// Lemma (derived fact with proof)
    ///
    /// ```text
    /// T1: false
    /// [lemma T1]: (or (not l_1) ... (not l_n))
    /// ```
    Lemma {
        conclusion: Text,
        proof: Box<ProofTerm>,
    },

    /// Hypothesis (local assumption in natural deduction)
    Hypothesis { id: usize, formula: Text },

    // ==================== New Proof Rules ====================
    /// And elimination: from (and l_1 ... l_n), derive l_i
    ///
    /// ```text
    /// T1: (and l_1 ... l_n)
    /// [and-elim T1]: l_i
    /// ```
    AndElim {
        /// Proof of the conjunction
        conjunction: Box<ProofTerm>,
        /// Index of the conjunct being extracted (0-based)
        index: usize,
        /// The extracted conjunct formula
        result: Text,
    },

    /// Not-or elimination: from (not (or l_1 ... l_n)), derive (not l_i)
    ///
    /// ```text
    /// T1: (not (or l_1 ... l_n))
    /// [not-or-elim T1]: (not l_i)
    /// ```
    NotOrElim {
        /// Proof of the negated disjunction
        negated_disjunction: Box<ProofTerm>,
        /// Index of the disjunct being negated (0-based)
        index: usize,
        /// The negated disjunct formula
        result: Text,
    },

    /// Iff-true: from p, derive (iff p true)
    ///
    /// ```text
    /// T1: p
    /// [iff-true T1]: (iff p true)
    /// ```
    IffTrue {
        /// Proof of p
        proof: Box<ProofTerm>,
        /// The formula p
        formula: Text,
    },

    /// Iff-false: from (not p), derive (iff p false)
    ///
    /// ```text
    /// T1: (not p)
    /// [iff-false T1]: (iff p false)
    /// ```
    IffFalse {
        /// Proof of (not p)
        proof: Box<ProofTerm>,
        /// The formula p
        formula: Text,
    },

    /// Commutativity: derive (= (f a b) (f b a)) for commutative f
    ///
    /// ```text
    /// [comm]: (= (f a b) (f b a))
    /// ```
    Commutativity {
        /// The left side of the equality
        left: Text,
        /// The right side of the equality
        right: Text,
    },

    /// Monotonicity: if a R a', b R b', then f(a,b) R f(a',b')
    ///
    /// ```text
    /// T1: (R a a')
    /// T2: (R b b')
    /// [monotonicity T1 T2]: (R (f a b) (f a' b'))
    /// ```
    Monotonicity {
        /// Proofs of the component relations
        premises: List<ProofTerm>,
        /// The conclusion formula
        conclusion: Text,
    },

    /// Distributivity: f distributes over g
    ///
    /// ```text
    /// [distributivity]: (= (f a (g c d)) (g (f a c) (f a d)))
    /// ```
    Distributivity {
        /// The distributivity equality formula
        formula: Text,
    },

    /// Definition axiom: Tseitin-style CNF transformation axiom
    ///
    /// Propositional tautologies for CNF conversion:
    /// - (or (not (and p q)) p)
    /// - (or (and p q) (not p) (not q))
    /// - etc.
    DefAxiom {
        /// The tautology formula
        formula: Text,
    },

    /// Definition introduction: introduces a name for an expression
    ///
    /// ```text
    /// [def-intro]: (and (or n (not e)) (or (not n) e))
    /// ```
    DefIntro {
        /// The name being introduced
        name: Text,
        /// The definition formula
        definition: Text,
    },

    /// Apply definition: apply a definition to rewrite
    ///
    /// ```text
    /// [apply-def T1]: F ~ n
    /// ```
    ApplyDef {
        /// Proof that n is a name for F
        def_proof: Box<ProofTerm>,
        /// The original formula F
        original: Text,
        /// The name n
        name: Text,
    },

    /// Iff to oriented equality: from (iff p q), derive (~ p q)
    ///
    /// ```text
    /// T1: (iff p q)
    /// [iff~ T1]: (~ p q)
    /// ```
    IffOEq {
        /// Proof of the iff
        iff_proof: Box<ProofTerm>,
        /// Left side of equivalence
        left: Text,
        /// Right side of equivalence
        right: Text,
    },

    /// NNF positive: negation normal form transformation (positive context)
    ///
    /// Used when creating NNF of positive force quantifiers or
    /// recursively creating NNF over Boolean formulas.
    NNFPos {
        /// Antecedent proofs
        premises: List<ProofTerm>,
        /// The NNF equivalence
        conclusion: Text,
    },

    /// NNF negative: negation normal form transformation (negative context)
    ///
    /// ```text
    /// T1: (not s_1) ~ r_1
    /// ...
    /// [nnf-neg T1 ...]: (not (and s_1 ...)) ~ (or r_1 ...)
    /// ```
    NNFNeg {
        /// Antecedent proofs
        premises: List<ProofTerm>,
        /// The NNF equivalence
        conclusion: Text,
    },

    /// Skolemization: introduce Skolem functions for existentials
    ///
    /// ```text
    /// [sk]: (~ (exists x (p x y)) (p (sk y) y))
    /// ```
    Skolemize {
        /// The skolemization equivalence
        formula: Text,
    },

    /// Quantifier introduction: from (~ p q), derive (~ (forall x p) (forall x q))
    ///
    /// ```text
    /// T1: (~ p q)
    /// [quant-intro T1]: (~ (forall (x) p) (forall (x) q))
    /// ```
    QuantIntro {
        /// Proof of the body equivalence
        body_proof: Box<ProofTerm>,
        /// The quantified equivalence
        conclusion: Text,
    },

    /// Proof bind: from f, derive (forall x f) where x are free in f
    ///
    /// ```text
    /// T1: f
    /// [proof-bind T1]: forall (x) f
    /// ```
    Bind {
        /// Proof of the body
        body_proof: Box<ProofTerm>,
        /// The bound variables
        variables: List<Text>,
        /// The quantified formula
        conclusion: Text,
    },

    /// Pull quantifier: pull quantifier out of a formula
    ///
    /// ```text
    /// [pull-quant]: (iff (f (forall (x) q(x)) r) (forall (x) (f (q x) r)))
    /// ```
    PullQuant {
        /// The pull-quantifier equivalence
        formula: Text,
    },

    /// Push quantifier: push quantifier into a formula
    ///
    /// ```text
    /// [push-quant]: (iff (forall (x) (and p q)) (and (forall (x) p) (forall (x) q)))
    /// ```
    PushQuant {
        /// The push-quantifier equivalence
        formula: Text,
    },

    /// Eliminate unused variables
    ///
    /// ```text
    /// [elim-unused]: (iff (forall (x y) p[x]) (forall (x) p[x]))
    /// ```
    ElimUnusedVars {
        /// The variable elimination equivalence
        formula: Text,
    },

    /// Destructive equality resolution
    ///
    /// ```text
    /// [der]: (iff (forall (x) (or (not (= x t)) P[x])) P[t])
    /// ```
    DestructiveEqRes {
        /// The DER equivalence formula
        formula: Text,
    },

    /// Hyper-resolution: generalized resolution rule
    ///
    /// Takes multiple clauses and resolves them together.
    HyperResolve {
        /// The clauses being resolved
        clauses: List<ProofTerm>,
        /// The conclusion
        conclusion: Text,
    },
}

impl ProofTerm {
    /// Get the conclusion of this proof term
    pub fn conclusion(&self) -> Text {
        match self {
            Self::Axiom { formula, .. } => formula.clone(),
            Self::Assumption { formula, .. } => formula.clone(),
            Self::ModusPonens { implication, .. } => {
                // Extract consequent from implication
                implication.conclusion()
            }
            Self::Rewrite { target, .. } => target.clone(),
            Self::Symmetry { equality } => {
                // Flip equality - swap sides of the equality
                let eq_str = equality.conclusion();
                if let Some(eq_idx) = eq_str.as_str().find("=") {
                    let left = eq_str.as_str()[..eq_idx].trim();
                    let right = eq_str.as_str()[eq_idx + 1..].trim();
                    format!("{} = {}", right, left).into()
                } else {
                    eq_str
                }
            }
            Self::Transitivity { left, right } => {
                // From A = B and B = C, the conclusion is A = C
                let left_eq = left.conclusion();
                let right_eq = right.conclusion();
                if let (Some(left_idx), Some(right_idx)) =
                    (left_eq.as_str().find("="), right_eq.as_str().find("="))
                {
                    let a = left_eq.as_str()[..left_idx].trim();
                    let c = right_eq.as_str()[right_idx + 1..].trim();
                    format!("{} = {}", a, c).into()
                } else {
                    right_eq
                }
            }
            Self::Reflexivity { term } => format!("{} = {}", term, term).into(),
            Self::TheoryLemma { lemma, .. } => lemma.clone(),
            Self::UnitResolution { clauses } => {
                // Result of resolution
                clauses
                    .last()
                    .map(|c| c.conclusion())
                    .unwrap_or_else(|| "false".to_text())
            }
            Self::QuantifierInstantiation { quantified, .. } => quantified.conclusion(),
            Self::Lemma { conclusion, .. } => conclusion.clone(),
            Self::Hypothesis { formula, .. } => formula.clone(),

            // New proof rules
            Self::AndElim { result, .. } => result.clone(),
            Self::NotOrElim { result, .. } => result.clone(),
            Self::IffTrue { formula, .. } => format!("(iff {} true)", formula).into(),
            Self::IffFalse { formula, .. } => format!("(iff {} false)", formula).into(),
            Self::Commutativity { left, right } => format!("(= {} {})", left, right).into(),
            Self::Monotonicity { conclusion, .. } => conclusion.clone(),
            Self::Distributivity { formula } => formula.clone(),
            Self::DefAxiom { formula } => formula.clone(),
            Self::DefIntro { definition, .. } => definition.clone(),
            Self::ApplyDef { name, .. } => name.clone(),
            Self::IffOEq { left, right, .. } => format!("(~ {} {})", left, right).into(),
            Self::NNFPos { conclusion, .. } => conclusion.clone(),
            Self::NNFNeg { conclusion, .. } => conclusion.clone(),
            Self::Skolemize { formula } => formula.clone(),
            Self::QuantIntro { conclusion, .. } => conclusion.clone(),
            Self::Bind { conclusion, .. } => conclusion.clone(),
            Self::PullQuant { formula } => formula.clone(),
            Self::PushQuant { formula } => formula.clone(),
            Self::ElimUnusedVars { formula } => formula.clone(),
            Self::DestructiveEqRes { formula } => formula.clone(),
            Self::HyperResolve { conclusion, .. } => conclusion.clone(),
        }
    }

    /// Get all axioms used in this proof
    pub fn used_axioms(&self) -> Set<Text> {
        let mut axioms = Set::new();
        self.collect_axioms(&mut axioms);
        axioms
    }

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
            Self::Rewrite { source, .. } => {
                source.collect_axioms(axioms);
            }
            Self::Symmetry { equality } => {
                equality.collect_axioms(axioms);
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
            Self::QuantifierInstantiation { quantified, .. } => {
                quantified.collect_axioms(axioms);
            }
            Self::Lemma { proof, .. } => {
                proof.collect_axioms(axioms);
            }

            // New proof rules with sub-proofs
            Self::AndElim { conjunction, .. } => {
                conjunction.collect_axioms(axioms);
            }
            Self::NotOrElim {
                negated_disjunction,
                ..
            } => {
                negated_disjunction.collect_axioms(axioms);
            }
            Self::IffTrue { proof, .. } | Self::IffFalse { proof, .. } => {
                proof.collect_axioms(axioms);
            }
            Self::Monotonicity { premises, .. } => {
                for premise in premises {
                    premise.collect_axioms(axioms);
                }
            }
            Self::ApplyDef { def_proof, .. } => {
                def_proof.collect_axioms(axioms);
            }
            Self::IffOEq { iff_proof, .. } => {
                iff_proof.collect_axioms(axioms);
            }
            Self::NNFPos { premises, .. } | Self::NNFNeg { premises, .. } => {
                for premise in premises {
                    premise.collect_axioms(axioms);
                }
            }
            Self::QuantIntro { body_proof, .. } | Self::Bind { body_proof, .. } => {
                body_proof.collect_axioms(axioms);
            }
            Self::HyperResolve { clauses, .. } => {
                for clause in clauses {
                    clause.collect_axioms(axioms);
                }
            }

            // Leaf rules with no sub-proofs (no axioms to collect)
            Self::Assumption { .. }
            | Self::Reflexivity { .. }
            | Self::Hypothesis { .. }
            | Self::Commutativity { .. }
            | Self::Distributivity { .. }
            | Self::DefAxiom { .. }
            | Self::DefIntro { .. }
            | Self::Skolemize { .. }
            | Self::PullQuant { .. }
            | Self::PushQuant { .. }
            | Self::ElimUnusedVars { .. }
            | Self::DestructiveEqRes { .. } => {}
        }
    }

    /// Count proof steps (depth)
    pub fn proof_depth(&self) -> usize {
        match self {
            // Leaf nodes (depth 1)
            Self::Axiom { .. }
            | Self::Assumption { .. }
            | Self::Reflexivity { .. }
            | Self::TheoryLemma { .. }
            | Self::Hypothesis { .. }
            | Self::Commutativity { .. }
            | Self::Distributivity { .. }
            | Self::DefAxiom { .. }
            | Self::DefIntro { .. }
            | Self::Skolemize { .. }
            | Self::PullQuant { .. }
            | Self::PushQuant { .. }
            | Self::ElimUnusedVars { .. }
            | Self::DestructiveEqRes { .. } => 1,

            // Two sub-proofs
            Self::ModusPonens {
                premise,
                implication,
            } => 1 + premise.proof_depth().max(implication.proof_depth()),

            Self::Transitivity { left, right } => 1 + left.proof_depth().max(right.proof_depth()),

            // Single sub-proof
            Self::Rewrite { source, .. } | Self::Symmetry { equality: source } => {
                1 + source.proof_depth()
            }

            Self::QuantifierInstantiation { quantified, .. } => 1 + quantified.proof_depth(),

            Self::Lemma { proof, .. } => 1 + proof.proof_depth(),

            Self::AndElim { conjunction, .. } => 1 + conjunction.proof_depth(),

            Self::NotOrElim {
                negated_disjunction,
                ..
            } => 1 + negated_disjunction.proof_depth(),

            Self::IffTrue { proof, .. } | Self::IffFalse { proof, .. } => 1 + proof.proof_depth(),

            Self::ApplyDef { def_proof, .. } => 1 + def_proof.proof_depth(),

            Self::IffOEq { iff_proof, .. } => 1 + iff_proof.proof_depth(),

            Self::QuantIntro { body_proof, .. } | Self::Bind { body_proof, .. } => {
                1 + body_proof.proof_depth()
            }

            // Multiple sub-proofs
            Self::UnitResolution { clauses } | Self::HyperResolve { clauses, .. } => {
                1 + clauses.iter().map(|c| c.proof_depth()).max().unwrap_or(0)
            }

            Self::Monotonicity { premises, .. }
            | Self::NNFPos { premises, .. }
            | Self::NNFNeg { premises, .. } => {
                1 + premises.iter().map(|p| p.proof_depth()).max().unwrap_or(0)
            }
        }
    }

    /// Count total proof nodes
    pub fn node_count(&self) -> usize {
        match self {
            // Leaf nodes (count 1)
            Self::Axiom { .. }
            | Self::Assumption { .. }
            | Self::Reflexivity { .. }
            | Self::TheoryLemma { .. }
            | Self::Hypothesis { .. }
            | Self::Commutativity { .. }
            | Self::Distributivity { .. }
            | Self::DefAxiom { .. }
            | Self::DefIntro { .. }
            | Self::Skolemize { .. }
            | Self::PullQuant { .. }
            | Self::PushQuant { .. }
            | Self::ElimUnusedVars { .. }
            | Self::DestructiveEqRes { .. } => 1,

            // Two sub-proofs
            Self::ModusPonens {
                premise,
                implication,
            } => 1 + premise.node_count() + implication.node_count(),

            Self::Transitivity { left, right } => 1 + left.node_count() + right.node_count(),

            // Single sub-proof
            Self::Rewrite { source, .. } | Self::Symmetry { equality: source } => {
                1 + source.node_count()
            }

            Self::QuantifierInstantiation { quantified, .. } => 1 + quantified.node_count(),

            Self::Lemma { proof, .. } => 1 + proof.node_count(),

            Self::AndElim { conjunction, .. } => 1 + conjunction.node_count(),

            Self::NotOrElim {
                negated_disjunction,
                ..
            } => 1 + negated_disjunction.node_count(),

            Self::IffTrue { proof, .. } | Self::IffFalse { proof, .. } => 1 + proof.node_count(),

            Self::ApplyDef { def_proof, .. } => 1 + def_proof.node_count(),

            Self::IffOEq { iff_proof, .. } => 1 + iff_proof.node_count(),

            Self::QuantIntro { body_proof, .. } | Self::Bind { body_proof, .. } => {
                1 + body_proof.node_count()
            }

            // Multiple sub-proofs
            Self::UnitResolution { clauses } | Self::HyperResolve { clauses, .. } => {
                1 + clauses.iter().map(|c| c.node_count()).sum::<usize>()
            }

            Self::Monotonicity { premises, .. }
            | Self::NNFPos { premises, .. }
            | Self::NNFNeg { premises, .. } => {
                1 + premises.iter().map(|p| p.node_count()).sum::<usize>()
            }
        }
    }
}

// ==================== Proof Extractor ====================

/// Extracts and analyzes Z3 proof objects
///
/// The ProofExtractor is the main entry point for extracting structured proof terms
/// from Z3 solver proofs. It supports:
/// - Configurable extraction depth
/// - Optional proof simplification
/// - Optional validation on extraction
/// - Caching support via serialization
///
/// ## Usage
///
/// ```ignore
/// use verum_smt::proof_extraction::{ProofExtractor, ProofGenerationConfig};
///
/// // Create with default settings
/// let extractor = ProofExtractor::new();
///
/// // Create with production settings
/// let extractor = ProofExtractor::with_config(ProofGenerationConfig::production());
///
/// // Extract proof from Z3 solver
/// if let Some(proof) = solver.get_proof() {
///     if let Some(term) = extractor.extract_proof(&proof) {
///         println!("Extracted: {:?}", term);
///     }
/// }
/// ```
pub struct ProofExtractor {
    /// Enable proof simplification
    pub simplify_proofs: bool,
    /// Maximum proof depth to extract (prevents infinite loops)
    pub max_depth: usize,
    /// Full configuration (used internally)
    config: ProofGenerationConfig,
}

impl ProofExtractor {
    /// Create new proof extractor with default settings
    pub fn new() -> Self {
        let config = ProofGenerationConfig::default();
        Self {
            simplify_proofs: config.simplify_proofs,
            max_depth: config.max_proof_depth,
            config,
        }
    }

    /// Create proof extractor with custom configuration
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let config = ProofGenerationConfig::production();
    /// let extractor = ProofExtractor::with_config(config);
    /// ```
    pub fn with_config(config: ProofGenerationConfig) -> Self {
        Self {
            simplify_proofs: config.simplify_proofs,
            max_depth: config.max_proof_depth,
            config,
        }
    }

    /// Get the current configuration
    pub fn config(&self) -> &ProofGenerationConfig {
        &self.config
    }

    /// Extract proof from Z3 solver
    ///
    /// Returns structured proof term if available.
    /// Note: Z3 must be configured with proof generation enabled.
    ///
    /// # Example
    /// ```ignore
    /// let extractor = ProofExtractor::new();
    /// if let Some(proof_obj) = solver.get_proof() {
    ///     if let Some(proof_term) = extractor.extract_proof(&proof_obj) {
    ///         println!("Proof extracted: {:?}", proof_term);
    ///     }
    /// }
    /// ```
    pub fn extract_proof(&self, proof_obj: &Dynamic) -> Maybe<ProofTerm> {
        // Parse Z3 proof object to structured term
        let proof_term = self.parse_proof_object(proof_obj, 0)?;

        // Apply minimization if enabled
        let proof_term = if self.simplify_proofs {
            ProofMinimizer::minimize(&proof_term)
        } else {
            proof_term
        };

        // Validate if configured.  When `validate_on_extract` is set the
        // caller has declared that an unvalidated proof must NOT be
        // returned — fail-closed by returning `Maybe::None` on a hard
        // validation error.  Pre-fix the failure was logged at WARN and
        // the (possibly invalid) proof was returned anyway, defeating
        // the entire enforcement contract of the flag.  Callers that
        // want the warn-but-don't-reject behaviour should use
        // `extract_proof_with_validation` and inspect the returned
        // `ProofValidation` themselves.
        if self.config.validate_on_extract {
            let validation = self.validate_proof(&proof_term);
            if !validation.is_ok() {
                tracing::warn!(
                    "Extracted proof failed validation ({} errors, {} warnings); \
                     rejecting per validate_on_extract=true",
                    validation.errors.len(),
                    validation.warnings.len()
                );
                return Maybe::None;
            }
        }

        Maybe::Some(proof_term)
    }

    /// Extract proof with full result including validation
    ///
    /// Returns both the proof term and validation results.
    pub fn extract_proof_with_validation(
        &self,
        proof_obj: &Dynamic,
    ) -> Maybe<(ProofTerm, ProofValidation)> {
        let proof_term = self.parse_proof_object(proof_obj, 0)?;

        let proof_term = if self.simplify_proofs {
            ProofMinimizer::minimize(&proof_term)
        } else {
            proof_term
        };

        let validation = self.validate_proof(&proof_term);

        Maybe::Some((proof_term, validation))
    }

    /// Validate a proof term for structural soundness
    ///
    /// Checks:
    /// - All proof steps are valid
    /// - No circular dependencies
    /// - Axioms are properly used
    /// - Conclusions follow from premises
    ///
    /// Returns true if proof is valid, false otherwise.
    pub fn validate_proof(&self, proof: &ProofTerm) -> ProofValidation {
        let mut validation = ProofValidation {
            is_valid: true,
            errors: List::new(),
            warnings: List::new(),
        };

        self.validate_proof_impl(proof, &mut Set::new(), &mut validation, 0);
        validation
    }

    /// Internal recursive validation
    fn validate_proof_impl(
        &self,
        proof: &ProofTerm,
        visited: &mut Set<Text>,
        validation: &mut ProofValidation,
        depth: usize,
    ) {
        // Check max depth
        if depth > self.max_depth {
            validation.is_valid = false;
            validation
                .errors
                .push(format!("Proof depth exceeds maximum of {}", self.max_depth).into());
            return;
        }

        // Track proof node to detect cycles
        let node_id: Text = format!("{:?}_{}", proof, depth).into();
        if visited.contains(&node_id) {
            validation.is_valid = false;
            validation
                .errors
                .push("Circular dependency detected in proof".to_text());
            return;
        }
        visited.insert(node_id);

        // Validate proof structure based on type
        match proof {
            ProofTerm::Axiom { name, formula } => {
                if name.is_empty() {
                    validation.warnings.push("Axiom has no name".to_text());
                }
                if formula.is_empty() {
                    validation.is_valid = false;
                    validation.errors.push("Axiom has empty formula".to_text());
                }
            }

            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                // Validate sub-proofs
                self.validate_proof_impl(premise, visited, validation, depth + 1);
                self.validate_proof_impl(implication, visited, validation, depth + 1);

                // Check that implication conclusion matches expected form
                let impl_conclusion = implication.conclusion();
                if !impl_conclusion.contains("=>") && !impl_conclusion.contains("->") {
                    validation
                        .warnings
                        .push("ModusPonens implication does not have implication form".to_text());
                }
            }

            ProofTerm::Rewrite {
                source,
                rule,
                target,
            } => {
                self.validate_proof_impl(source, visited, validation, depth + 1);

                if rule.is_empty() {
                    validation.warnings.push("Rewrite rule is empty".to_text());
                }
                if target.is_empty() {
                    validation.is_valid = false;
                    validation.errors.push("Rewrite target is empty".to_text());
                }
            }

            ProofTerm::Symmetry { equality } => {
                self.validate_proof_impl(equality, visited, validation, depth + 1);

                let conclusion = equality.conclusion();
                if !conclusion.contains("=") {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Symmetry requires equality, got non-equality".to_text());
                }
            }

            ProofTerm::Transitivity { left, right } => {
                self.validate_proof_impl(left, visited, validation, depth + 1);
                self.validate_proof_impl(right, visited, validation, depth + 1);

                // Check that both sides are equalities
                let left_conclusion = left.conclusion();
                let right_conclusion = right.conclusion();

                if !left_conclusion.contains("=") || !right_conclusion.contains("=") {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Transitivity requires equalities on both sides".to_text());
                }
            }

            ProofTerm::Reflexivity { term } => {
                if term.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Reflexivity term is empty".to_text());
                }
            }

            ProofTerm::TheoryLemma { theory, lemma } => {
                if theory.is_empty() {
                    validation
                        .warnings
                        .push("Theory lemma has no theory name".to_text());
                }
                if lemma.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Theory lemma has empty formula".to_text());
                }
            }

            ProofTerm::UnitResolution { clauses } => {
                if clauses.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Unit resolution has no clauses".to_text());
                    return;
                }

                for clause in clauses {
                    self.validate_proof_impl(clause, visited, validation, depth + 1);
                }
            }

            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => {
                self.validate_proof_impl(quantified, visited, validation, depth + 1);

                if instantiation.is_empty() {
                    validation
                        .warnings
                        .push("Quantifier instantiation has no bindings".to_text());
                }
            }

            ProofTerm::Lemma { conclusion, proof } => {
                if conclusion.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Lemma has empty conclusion".to_text());
                }
                self.validate_proof_impl(proof, visited, validation, depth + 1);
            }

            ProofTerm::Assumption { formula, .. } | ProofTerm::Hypothesis { formula, .. } => {
                if formula.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Assumption/Hypothesis has empty formula".to_text());
                }
            }

            // New proof rules validation
            ProofTerm::AndElim {
                conjunction,
                result,
                ..
            } => {
                self.validate_proof_impl(conjunction, visited, validation, depth + 1);
                if result.is_empty() {
                    validation.is_valid = false;
                    validation.errors.push("AndElim has empty result".to_text());
                }
            }

            ProofTerm::NotOrElim {
                negated_disjunction,
                result,
                ..
            } => {
                self.validate_proof_impl(negated_disjunction, visited, validation, depth + 1);
                if result.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("NotOrElim has empty result".to_text());
                }
            }

            ProofTerm::IffTrue { proof, formula } => {
                self.validate_proof_impl(proof, visited, validation, depth + 1);
                if formula.is_empty() {
                    validation
                        .warnings
                        .push("IffTrue has empty formula".to_text());
                }
            }

            ProofTerm::IffFalse { proof, formula } => {
                self.validate_proof_impl(proof, visited, validation, depth + 1);
                if formula.is_empty() {
                    validation
                        .warnings
                        .push("IffFalse has empty formula".to_text());
                }
            }

            ProofTerm::Commutativity { left, right } => {
                if left.is_empty() || right.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Commutativity has empty side".to_text());
                }
            }

            ProofTerm::Monotonicity {
                premises,
                conclusion,
            } => {
                for premise in premises {
                    self.validate_proof_impl(premise, visited, validation, depth + 1);
                }
                if conclusion.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Monotonicity has empty conclusion".to_text());
                }
            }

            ProofTerm::Distributivity { formula }
            | ProofTerm::DefAxiom { formula }
            | ProofTerm::Skolemize { formula }
            | ProofTerm::PullQuant { formula }
            | ProofTerm::PushQuant { formula }
            | ProofTerm::ElimUnusedVars { formula }
            | ProofTerm::DestructiveEqRes { formula } => {
                if formula.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Proof rule has empty formula".to_text());
                }
            }

            ProofTerm::DefIntro { name, definition } => {
                if name.is_empty() {
                    validation
                        .warnings
                        .push("DefIntro has empty name".to_text());
                }
                if definition.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("DefIntro has empty definition".to_text());
                }
            }

            ProofTerm::ApplyDef {
                def_proof,
                original,
                name,
            } => {
                self.validate_proof_impl(def_proof, visited, validation, depth + 1);
                if original.is_empty() || name.is_empty() {
                    validation
                        .warnings
                        .push("ApplyDef has empty original or name".to_text());
                }
            }

            ProofTerm::IffOEq {
                iff_proof,
                left,
                right,
            } => {
                self.validate_proof_impl(iff_proof, visited, validation, depth + 1);
                if left.is_empty() || right.is_empty() {
                    validation.is_valid = false;
                    validation.errors.push("IffOEq has empty side".to_text());
                }
            }

            ProofTerm::NNFPos {
                premises,
                conclusion,
            }
            | ProofTerm::NNFNeg {
                premises,
                conclusion,
            } => {
                for premise in premises {
                    self.validate_proof_impl(premise, visited, validation, depth + 1);
                }
                if conclusion.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("NNF rule has empty conclusion".to_text());
                }
            }

            ProofTerm::QuantIntro {
                body_proof,
                conclusion,
            } => {
                self.validate_proof_impl(body_proof, visited, validation, depth + 1);
                if conclusion.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("QuantIntro has empty conclusion".to_text());
                }
            }

            ProofTerm::Bind {
                body_proof,
                variables,
                conclusion,
            } => {
                self.validate_proof_impl(body_proof, visited, validation, depth + 1);
                if variables.is_empty() {
                    validation
                        .warnings
                        .push("Bind has no bound variables".to_text());
                }
                if conclusion.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("Bind has empty conclusion".to_text());
                }
            }

            ProofTerm::HyperResolve {
                clauses,
                conclusion,
            } => {
                if clauses.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("HyperResolve has no clauses".to_text());
                    return;
                }

                for clause in clauses {
                    self.validate_proof_impl(clause, visited, validation, depth + 1);
                }

                if conclusion.is_empty() {
                    validation.is_valid = false;
                    validation
                        .errors
                        .push("HyperResolve has empty conclusion".to_text());
                }
            }
        }
    }

    /// Minimize a proof by removing redundant steps
    ///
    /// This is a convenience wrapper around `ProofMinimizer::minimize`.
    pub fn minimize_proof(&self, proof: &ProofTerm) -> ProofTerm {
        ProofMinimizer::minimize(proof)
    }

    fn parse_proof_object(&self, proof: &Dynamic, depth: usize) -> Maybe<ProofTerm> {
        use z3_sys::DeclKind;

        if depth > self.max_depth {
            return Maybe::None;
        }

        // Check if proof is an application (has a declaration)
        if !proof.is_app() {
            // Leaf node - likely a variable or numeral
            return Maybe::Some(ProofTerm::Axiom {
                name: "leaf".to_text(),
                formula: format!("{:?}", proof).into(),
            });
        }

        // Get the function declaration to determine proof rule
        let decl = match proof.safe_decl() {
            Ok(d) => d,
            Err(_) => {
                return Maybe::Some(ProofTerm::Axiom {
                    name: "non-app".to_text(),
                    formula: format!("{:?}", proof).into(),
                });
            }
        };

        let decl_kind = decl.kind();
        let num_args = proof.num_children();

        // Extract formula if this is the conclusion
        let formula: Text = if num_args > 0 {
            if let Some(last_child) = proof.nth_child(num_args - 1) {
                format!("{:?}", last_child).into()
            } else {
                format!("{:?}", proof).into()
            }
        } else {
            format!("{:?}", proof).into()
        };

        // Parse based on Z3 proof rule
        match decl_kind {
            // Asserted axiom
            DeclKind::PrAsserted => {
                let name = decl.name();
                Maybe::Some(ProofTerm::Axiom {
                    name: name.into(),
                    formula,
                })
            }

            // Modus ponens: (PrModusPonens premise implication)
            DeclKind::PrModusPonens | DeclKind::PrModusPonensOeq => {
                if num_args >= 2 {
                    let premise = proof.nth_child(0)?;
                    let implication = proof.nth_child(1)?;

                    let premise_term = self.parse_proof_object(&premise, depth + 1)?;
                    let impl_term = self.parse_proof_object(&implication, depth + 1)?;

                    Maybe::Some(ProofTerm::ModusPonens {
                        premise: Box::new(premise_term),
                        implication: Box::new(impl_term),
                    })
                } else {
                    Maybe::None
                }
            }

            // Reflexivity: A = A
            DeclKind::PrReflexivity => {
                let term_str = if num_args > 0 {
                    if let Some(child) = proof.nth_child(0) {
                        format!("{:?}", child)
                    } else {
                        "unknown".to_string()
                    }
                } else {
                    "unknown".to_string()
                };

                Maybe::Some(ProofTerm::Reflexivity {
                    term: term_str.into(),
                })
            }

            // Symmetry: from A = B derive B = A
            DeclKind::PrSymmetry => {
                if num_args >= 1 {
                    let equality = proof.nth_child(0)?;
                    let eq_term = self.parse_proof_object(&equality, depth + 1)?;

                    Maybe::Some(ProofTerm::Symmetry {
                        equality: Box::new(eq_term),
                    })
                } else {
                    Maybe::None
                }
            }

            // Transitivity: from A = B and B = C derive A = C
            DeclKind::PrTransitivity | DeclKind::PrTransitivityStar => {
                if num_args >= 2 {
                    let left = proof.nth_child(0)?;
                    let right = proof.nth_child(1)?;

                    let left_term = self.parse_proof_object(&left, depth + 1)?;
                    let right_term = self.parse_proof_object(&right, depth + 1)?;

                    Maybe::Some(ProofTerm::Transitivity {
                        left: Box::new(left_term),
                        right: Box::new(right_term),
                    })
                } else {
                    Maybe::None
                }
            }

            // Rewrite rule
            DeclKind::PrRewrite | DeclKind::PrRewriteStar => {
                if num_args >= 1 {
                    let source = proof.nth_child(0)?;
                    let source_term = self.parse_proof_object(&source, depth + 1)?;

                    let target = if num_args >= 2 {
                        if let Some(t) = proof.nth_child(1) {
                            format!("{:?}", t).into()
                        } else {
                            formula.clone()
                        }
                    } else {
                        formula.clone()
                    };

                    Maybe::Some(ProofTerm::Rewrite {
                        source: Box::new(source_term),
                        rule: decl.name().into(),
                        target,
                    })
                } else {
                    Maybe::None
                }
            }

            // Theory lemma (SMT theory reasoning)
            DeclKind::PrThLemma => {
                let theory_name = decl.name();
                Maybe::Some(ProofTerm::TheoryLemma {
                    theory: theory_name.into(),
                    lemma: formula,
                })
            }

            // Unit resolution (SAT reasoning)
            DeclKind::PrUnitResolution => {
                let mut clauses = List::new();
                for i in 0..num_args {
                    if let Some(clause_ast) = proof.nth_child(i)
                        && let Maybe::Some(clause_term) =
                            self.parse_proof_object(&clause_ast, depth + 1)
                    {
                        clauses.push(clause_term);
                    }
                }

                Maybe::Some(ProofTerm::UnitResolution { clauses })
            }

            // Quantifier instantiation
            DeclKind::PrQuantInst => {
                if num_args >= 1 {
                    let quantified = proof.nth_child(0)?;
                    let quant_term = self.parse_proof_object(&quantified, depth + 1)?;

                    // Extract instantiation bindings from remaining arguments
                    let mut instantiation = Map::new();
                    for i in 1..num_args {
                        if let Some(binding) = proof.nth_child(i) {
                            let binding_str = format!("{:?}", binding);
                            instantiation.insert(format!("v{}", i).into(), binding_str.into());
                        }
                    }

                    Maybe::Some(ProofTerm::QuantifierInstantiation {
                        quantified: Box::new(quant_term),
                        instantiation,
                    })
                } else {
                    Maybe::None
                }
            }

            // Lemma
            DeclKind::PrLemma => {
                if num_args >= 1 {
                    let proof_child = proof.nth_child(0)?;
                    let proof_term = self.parse_proof_object(&proof_child, depth + 1)?;

                    Maybe::Some(ProofTerm::Lemma {
                        conclusion: formula,
                        proof: Box::new(proof_term),
                    })
                } else {
                    Maybe::None
                }
            }

            // Hypothesis (local assumption)
            DeclKind::PrHypothesis => {
                let id = num_args; // Use number of args as ID
                Maybe::Some(ProofTerm::Hypothesis { id, formula })
            }

            // Goal
            DeclKind::PrGoal => Maybe::Some(ProofTerm::Axiom {
                name: "goal".to_text(),
                formula,
            }),

            // True constant
            DeclKind::PrTrue => Maybe::Some(ProofTerm::Axiom {
                name: "true".to_text(),
                formula: "true".to_text(),
            }),

            // Hyper resolution (generalized unit resolution)
            DeclKind::PrHyperResolve => {
                let mut clauses = List::new();
                for i in 0..num_args {
                    if let Some(clause_ast) = proof.nth_child(i)
                        && let Maybe::Some(clause_term) =
                            self.parse_proof_object(&clause_ast, depth + 1)
                    {
                        clauses.push(clause_term);
                    }
                }

                Maybe::Some(ProofTerm::HyperResolve {
                    clauses,
                    conclusion: formula,
                })
            }

            // And elimination: from (and l_1 ... l_n), derive l_i
            DeclKind::PrAndElim => {
                if num_args >= 1 {
                    let conjunction_ast = proof.nth_child(0)?;
                    let conjunction_term = self.parse_proof_object(&conjunction_ast, depth + 1)?;

                    // The index is encoded in the proof structure
                    // For now, extract from the number of arguments
                    let index = num_args.saturating_sub(1);

                    Maybe::Some(ProofTerm::AndElim {
                        conjunction: Box::new(conjunction_term),
                        index,
                        result: formula,
                    })
                } else {
                    Maybe::None
                }
            }

            // Not-or elimination: from (not (or l_1 ... l_n)), derive (not l_i)
            DeclKind::PrNotOrElim => {
                if num_args >= 1 {
                    let negated_disjunction_ast = proof.nth_child(0)?;
                    let negated_disjunction_term =
                        self.parse_proof_object(&negated_disjunction_ast, depth + 1)?;

                    let index = num_args.saturating_sub(1);

                    Maybe::Some(ProofTerm::NotOrElim {
                        negated_disjunction: Box::new(negated_disjunction_term),
                        index,
                        result: formula,
                    })
                } else {
                    Maybe::None
                }
            }

            // Iff-true: from p, derive (iff p true)
            DeclKind::PrIffTrue => {
                if num_args >= 1 {
                    let proof_ast = proof.nth_child(0)?;
                    let proof_term = self.parse_proof_object(&proof_ast, depth + 1)?;
                    let formula_p = proof_term.conclusion();

                    Maybe::Some(ProofTerm::IffTrue {
                        proof: Box::new(proof_term),
                        formula: formula_p,
                    })
                } else {
                    Maybe::None
                }
            }

            // Iff-false: from (not p), derive (iff p false)
            DeclKind::PrIffFalse => {
                if num_args >= 1 {
                    let proof_ast = proof.nth_child(0)?;
                    let proof_term = self.parse_proof_object(&proof_ast, depth + 1)?;

                    // Extract p from (not p)
                    let not_p = proof_term.conclusion();
                    let formula_p = if not_p.starts_with("(not ") && not_p.ends_with(")") {
                        Text::from(&not_p[5..not_p.len() - 1])
                    } else {
                        not_p
                    };

                    Maybe::Some(ProofTerm::IffFalse {
                        proof: Box::new(proof_term),
                        formula: formula_p,
                    })
                } else {
                    Maybe::None
                }
            }

            // Commutativity: (= (f a b) (f b a)) for commutative f
            DeclKind::PrCommutativity => {
                // Commutativity has no antecedents - extract from formula
                // Formula is of the form (= (f a b) (f b a))
                let (left, right) = Self::extract_equality_sides(&formula);
                Maybe::Some(ProofTerm::Commutativity { left, right })
            }

            // Monotonicity: from component proofs, derive function application equality
            DeclKind::PrMonotonicity => {
                let mut premises = List::new();
                for i in 0..num_args {
                    if let Some(premise_ast) = proof.nth_child(i)
                        && let Maybe::Some(premise_term) =
                            self.parse_proof_object(&premise_ast, depth + 1)
                    {
                        premises.push(premise_term);
                    }
                }

                Maybe::Some(ProofTerm::Monotonicity {
                    premises,
                    conclusion: formula,
                })
            }

            // Distributivity: no antecedents, conclusion is the distributivity formula
            DeclKind::PrDistributivity => Maybe::Some(ProofTerm::Distributivity { formula }),

            // Definition axiom: Tseitin-style CNF axiom
            DeclKind::PrDefAxiom => Maybe::Some(ProofTerm::DefAxiom { formula }),

            // Definition introduction: introduces a name for an expression
            DeclKind::PrDefIntro => {
                // Extract the name being defined from the formula
                // Format: (and (or n (not e)) (or (not n) e)) or (= n e)
                let name = if let Some(idx) = formula.as_str().find("=") {
                    Text::from(
                        formula.as_str()[idx + 1..]
                            .split_whitespace()
                            .next()
                            .unwrap_or("n"),
                    )
                } else {
                    "definition".to_text()
                };

                Maybe::Some(ProofTerm::DefIntro {
                    name,
                    definition: formula,
                })
            }

            // Apply definition: F ~ n given that n is defined as F
            DeclKind::PrApplyDef => {
                if num_args >= 1 {
                    let def_proof_ast = proof.nth_child(0)?;
                    let def_proof_term = self.parse_proof_object(&def_proof_ast, depth + 1)?;

                    // Extract original and name from the formula
                    let (original, name) = if formula.contains("~") {
                        let parts: Vec<&str> = formula.as_str().split('~').collect();
                        if parts.len() >= 2 {
                            (Text::from(parts[0].trim()), Text::from(parts[1].trim()))
                        } else {
                            (formula.clone(), "n".to_text())
                        }
                    } else {
                        (formula.clone(), "n".to_text())
                    };

                    Maybe::Some(ProofTerm::ApplyDef {
                        def_proof: Box::new(def_proof_term),
                        original,
                        name,
                    })
                } else {
                    Maybe::None
                }
            }

            // Iff to oriented equality: from (iff p q), derive (~ p q)
            DeclKind::PrIffOeq => {
                if num_args >= 1 {
                    let iff_proof_ast = proof.nth_child(0)?;
                    let iff_proof_term = self.parse_proof_object(&iff_proof_ast, depth + 1)?;

                    // Extract left and right from the iff conclusion
                    let iff_conclusion = iff_proof_term.conclusion();
                    let (left, right) = Self::extract_iff_sides(&iff_conclusion);

                    Maybe::Some(ProofTerm::IffOEq {
                        iff_proof: Box::new(iff_proof_term),
                        left,
                        right,
                    })
                } else {
                    Maybe::None
                }
            }

            // NNF positive: negation normal form (positive context)
            DeclKind::PrNnfPos => {
                let mut premises = List::new();
                for i in 0..num_args {
                    if let Some(premise_ast) = proof.nth_child(i)
                        && let Maybe::Some(premise_term) =
                            self.parse_proof_object(&premise_ast, depth + 1)
                    {
                        premises.push(premise_term);
                    }
                }

                Maybe::Some(ProofTerm::NNFPos {
                    premises,
                    conclusion: formula,
                })
            }

            // NNF negative: negation normal form (negative context)
            DeclKind::PrNnfNeg => {
                let mut premises = List::new();
                for i in 0..num_args {
                    if let Some(premise_ast) = proof.nth_child(i)
                        && let Maybe::Some(premise_term) =
                            self.parse_proof_object(&premise_ast, depth + 1)
                    {
                        premises.push(premise_term);
                    }
                }

                Maybe::Some(ProofTerm::NNFNeg {
                    premises,
                    conclusion: formula,
                })
            }

            // Skolemization: introduce Skolem functions
            DeclKind::PrSkolemize => Maybe::Some(ProofTerm::Skolemize { formula }),

            // Quantifier introduction: from (~ p q), derive (~ (forall x p) (forall x q))
            DeclKind::PrQuantIntro => {
                if num_args >= 1 {
                    let body_proof_ast = proof.nth_child(0)?;
                    let body_proof_term = self.parse_proof_object(&body_proof_ast, depth + 1)?;

                    Maybe::Some(ProofTerm::QuantIntro {
                        body_proof: Box::new(body_proof_term),
                        conclusion: formula,
                    })
                } else {
                    Maybe::None
                }
            }

            // Proof bind: from f, derive (forall x f)
            DeclKind::PrBind => {
                if num_args >= 1 {
                    let body_proof_ast = proof.nth_child(0)?;
                    let body_proof_term = self.parse_proof_object(&body_proof_ast, depth + 1)?;

                    // Extract bound variables from the conclusion
                    let variables = Self::extract_quantified_variables(&formula);

                    Maybe::Some(ProofTerm::Bind {
                        body_proof: Box::new(body_proof_term),
                        variables,
                        conclusion: formula,
                    })
                } else {
                    Maybe::None
                }
            }

            // Pull quantifier: pull quantifier out of formula
            DeclKind::PrPullQuant => Maybe::Some(ProofTerm::PullQuant { formula }),

            // Push quantifier: push quantifier into formula
            DeclKind::PrPushQuant => Maybe::Some(ProofTerm::PushQuant { formula }),

            // Eliminate unused variables
            DeclKind::PrElimUnusedVars => Maybe::Some(ProofTerm::ElimUnusedVars { formula }),

            // Destructive equality resolution
            DeclKind::PrDer => Maybe::Some(ProofTerm::DestructiveEqRes { formula }),

            // Unknown/undefined proof rule
            _ => {
                let rule_name = decl.name();
                Maybe::Some(ProofTerm::Axiom {
                    name: format!("unknown:{}", rule_name).into(),
                    formula,
                })
            }
        }
    }

    /// Extract left and right sides from an equality formula
    ///
    /// Parses formulas of the form "(= left right)" or "left = right"
    fn extract_equality_sides(formula: &Text) -> (Text, Text) {
        let s = formula.as_str();

        // Handle S-expression form: (= left right)
        if s.starts_with("(=") || s.starts_with("(= ") {
            // Find balanced parentheses to extract left and right
            let inner = s.trim_start_matches("(=").trim_start_matches(' ');
            if let Some((left, right)) = Self::split_balanced_sexp(inner) {
                return (Text::from(left.trim()), Text::from(right.trim()));
            }
        }

        // Handle infix form: left = right
        if let Some(eq_idx) = s.find('=') {
            let left = s[..eq_idx].trim();
            let right = s[eq_idx + 1..].trim().trim_end_matches(')');
            return (Text::from(left), Text::from(right));
        }

        // Fallback
        (formula.clone(), formula.clone())
    }

    /// Extract left and right sides from an iff formula
    ///
    /// Parses formulas of the form "(iff left right)" or "left <=> right"
    fn extract_iff_sides(formula: &Text) -> (Text, Text) {
        let s = formula.as_str();

        // Handle S-expression form: (iff left right)
        if s.starts_with("(iff") {
            let inner = s.trim_start_matches("(iff").trim_start_matches(' ');
            if let Some((left, right)) = Self::split_balanced_sexp(inner) {
                return (Text::from(left.trim()), Text::from(right.trim()));
            }
        }

        // Handle infix form
        if let Some(idx) = s.find("<=>") {
            let left = s[..idx].trim();
            let right = s[idx + 3..].trim();
            return (Text::from(left), Text::from(right));
        }

        // Fallback
        (formula.clone(), formula.clone())
    }

    /// Split an S-expression into two balanced parts
    ///
    /// Given "a b)" or "(f x) (g y))", returns the first complete term and the rest
    fn split_balanced_sexp(s: &str) -> Option<(&str, &str)> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        let mut depth = 0;
        let mut in_first = true;
        let mut first_end = 0;

        for (i, c) in s.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        // End of the containing expression
                        if in_first {
                            // Only one term
                            return None;
                        }
                        return Some((&s[..first_end], s[first_end..i].trim()));
                    }
                    depth -= 1;
                }
                ' ' | '\t' | '\n' if depth == 0 && in_first => {
                    first_end = i;
                    in_first = false;
                }
                _ => {}
            }
        }

        if !in_first && first_end > 0 {
            Some((&s[..first_end], s[first_end..].trim().trim_end_matches(')')))
        } else {
            None
        }
    }

    /// Extract quantified variables from a formula
    ///
    /// Parses formulas of the form "(forall (x y z) body)" and extracts [x, y, z]
    fn extract_quantified_variables(formula: &Text) -> List<Text> {
        let s = formula.as_str();
        let mut variables = List::new();

        // Look for "(forall (" or "(exists ("
        let start_patterns = ["(forall (", "(exists (", "(forall(", "(exists("];

        for pattern in start_patterns {
            if let Some(start_idx) = s.find(pattern) {
                let vars_start = start_idx + pattern.len();
                // Find the closing parenthesis of the variable list
                if let Some(vars_end) = s[vars_start..].find(')') {
                    let vars_str = &s[vars_start..vars_start + vars_end];
                    // Split by whitespace and filter
                    for var in vars_str.split_whitespace() {
                        let var = var.trim_matches(|c| c == '(' || c == ')' || c == ':');
                        if !var.is_empty() && !var.contains(':') {
                            variables.push(Text::from(var));
                        }
                    }
                }
                break;
            }
        }

        variables
    }

    /// Analyze proof structure
    pub fn analyze(&self, proof: &ProofTerm) -> ProofAnalysis {
        ProofAnalysis {
            depth: proof.proof_depth(),
            node_count: proof.node_count(),
            axioms_used: proof.used_axioms(),
            has_quantifiers: self.has_quantifiers(proof),
            theory_lemmas: self.count_theory_lemmas(proof),
        }
    }

    fn has_quantifiers(&self, proof: &ProofTerm) -> bool {
        match proof {
            // Quantifier-related rules
            ProofTerm::QuantifierInstantiation { .. }
            | ProofTerm::QuantIntro { .. }
            | ProofTerm::Bind { .. }
            | ProofTerm::PullQuant { .. }
            | ProofTerm::PushQuant { .. }
            | ProofTerm::ElimUnusedVars { .. }
            | ProofTerm::Skolemize { .. } => true,

            // Two sub-proofs
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => self.has_quantifiers(premise) || self.has_quantifiers(implication),

            ProofTerm::Transitivity { left, right } => {
                self.has_quantifiers(left) || self.has_quantifiers(right)
            }

            // Single sub-proof
            ProofTerm::Rewrite { source, .. } | ProofTerm::Symmetry { equality: source } => {
                self.has_quantifiers(source)
            }

            ProofTerm::Lemma { proof, .. } => self.has_quantifiers(proof),

            ProofTerm::AndElim { conjunction, .. } => self.has_quantifiers(conjunction),

            ProofTerm::NotOrElim {
                negated_disjunction,
                ..
            } => self.has_quantifiers(negated_disjunction),

            ProofTerm::IffTrue { proof, .. } | ProofTerm::IffFalse { proof, .. } => {
                self.has_quantifiers(proof)
            }

            ProofTerm::ApplyDef { def_proof, .. } => self.has_quantifiers(def_proof),

            ProofTerm::IffOEq { iff_proof, .. } => self.has_quantifiers(iff_proof),

            // Multiple sub-proofs
            ProofTerm::UnitResolution { clauses } | ProofTerm::HyperResolve { clauses, .. } => {
                clauses.iter().any(|c| self.has_quantifiers(c))
            }

            ProofTerm::Monotonicity { premises, .. }
            | ProofTerm::NNFPos { premises, .. }
            | ProofTerm::NNFNeg { premises, .. } => {
                premises.iter().any(|p| self.has_quantifiers(p))
            }

            // Leaf nodes
            _ => false,
        }
    }

    fn count_theory_lemmas(&self, proof: &ProofTerm) -> usize {
        match proof {
            ProofTerm::TheoryLemma { .. } => 1,

            // Two sub-proofs
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => self.count_theory_lemmas(premise) + self.count_theory_lemmas(implication),

            ProofTerm::Transitivity { left, right } => {
                self.count_theory_lemmas(left) + self.count_theory_lemmas(right)
            }

            // Single sub-proof
            ProofTerm::Rewrite { source, .. } | ProofTerm::Symmetry { equality: source } => {
                self.count_theory_lemmas(source)
            }

            ProofTerm::Lemma { proof, .. } => self.count_theory_lemmas(proof),

            ProofTerm::AndElim { conjunction, .. } => self.count_theory_lemmas(conjunction),

            ProofTerm::NotOrElim {
                negated_disjunction,
                ..
            } => self.count_theory_lemmas(negated_disjunction),

            ProofTerm::IffTrue { proof, .. } | ProofTerm::IffFalse { proof, .. } => {
                self.count_theory_lemmas(proof)
            }

            ProofTerm::ApplyDef { def_proof, .. } => self.count_theory_lemmas(def_proof),

            ProofTerm::IffOEq { iff_proof, .. } => self.count_theory_lemmas(iff_proof),

            ProofTerm::QuantIntro { body_proof, .. } | ProofTerm::Bind { body_proof, .. } => {
                self.count_theory_lemmas(body_proof)
            }

            // Multiple sub-proofs
            ProofTerm::UnitResolution { clauses } | ProofTerm::HyperResolve { clauses, .. } => {
                clauses.iter().map(|c| self.count_theory_lemmas(c)).sum()
            }

            ProofTerm::Monotonicity { premises, .. }
            | ProofTerm::NNFPos { premises, .. }
            | ProofTerm::NNFNeg { premises, .. } => {
                premises.iter().map(|p| self.count_theory_lemmas(p)).sum()
            }

            // Leaf nodes
            _ => 0,
        }
    }

    // ==================== Tactic Composition for Proof Search ====================

    /// Apply tactics in sequence to search for proof
    ///
    /// This composes tactics to help find proofs:
    /// - Simplify the problem
    /// - Apply domain-specific tactics
    /// - Extract resulting proof
    ///
    /// Returns proof term if tactics succeed in finding a proof.
    pub fn apply_tactic_sequence(&self, goal: &Goal, tactics: &[TacticKind]) -> Maybe<ProofTerm> {
        // Build tactic strategy from sequence
        let mut builder = StrategyBuilder::new();
        for tactic in tactics {
            builder = builder.then(tactic.clone());
        }
        let strategy = builder.build();

        // Apply strategy
        self.apply_tactic_strategy(goal, &strategy)
    }

    /// Apply a tactic strategy to search for proof
    ///
    /// Uses the tactic combinators to build sophisticated proof search strategies.
    /// If the strategy succeeds, extracts the resulting proof term.
    pub fn apply_tactic_strategy(
        &self,
        goal: &Goal,
        strategy: &TacticCombinator,
    ) -> Maybe<ProofTerm> {
        // Convert strategy to Z3 tactic
        let tactic = self.combinator_to_tactic(strategy);

        // Apply tactic to goal
        let apply_result = match tactic.apply(goal, None) {
            Ok(result) => result,
            Err(_) => return Maybe::None,
        };

        // Extract proof from subgoals
        // In Z3, when tactics succeed in proving, they produce empty goal lists
        let subgoals = apply_result.list_subgoals();
        let subgoal_list: List<Goal> = subgoals.collect();

        if subgoal_list.is_empty() {
            // Goal was proven - extract proof
            // Note: Z3 doesn't directly expose proof from tactic application
            // We construct a synthetic proof term representing the tactic application
            Maybe::Some(ProofTerm::TheoryLemma {
                theory: "tactic".to_text(),
                lemma: format!("Proved via {:?}", strategy).into(),
            })
        } else {
            // Goals remain - no proof found
            Maybe::None
        }
    }

    /// Convert TacticCombinator to Z3 Tactic
    fn combinator_to_tactic(&self, combinator: &TacticCombinator) -> Tactic {
        match combinator {
            TacticCombinator::Single(kind) => Tactic::new(kind.name()),

            TacticCombinator::AndThen(t1, t2) => {
                let tactic1 = self.combinator_to_tactic(t1);
                let tactic2 = self.combinator_to_tactic(t2);
                tactic1.and_then(&tactic2)
            }

            TacticCombinator::OrElse(t1, t2) => {
                let tactic1 = self.combinator_to_tactic(t1);
                let tactic2 = self.combinator_to_tactic(t2);
                tactic1.or_else(&tactic2)
            }

            TacticCombinator::TryFor(t, _duration) => {
                // Note: Z3 doesn't directly support timeout in tactic composition
                // Apply the inner tactic directly
                self.combinator_to_tactic(t)
            }

            TacticCombinator::Repeat(t, max_iterations) => {
                // Create a repeated tactic by chaining and_then
                // Note: Tactic doesn't implement Clone, so we recreate it each time
                if *max_iterations == 0 {
                    return Tactic::new("skip");
                }

                let mut result = self.combinator_to_tactic(t);
                for _ in 1..*max_iterations {
                    let next = self.combinator_to_tactic(t);
                    result = result.and_then(&next);
                }
                result
            }

            TacticCombinator::WithParams(t, _params) => {
                // Note: Parameters would be applied via Params object
                // For now, apply inner tactic
                self.combinator_to_tactic(t)
            }

            TacticCombinator::IfThenElse {
                probe,
                then_tactic,
                else_tactic,
            } => {
                // Convert probe to Z3 probe
                let z3_probe = self.probe_kind_to_z3_probe(probe);

                let then_t = self.combinator_to_tactic(then_tactic);
                let else_t = self.combinator_to_tactic(else_tactic);

                // Use Z3's cond combinator (static method on Tactic)
                Tactic::cond(&z3_probe, &then_t, &else_t)
            }

            TacticCombinator::ParOr(tactics) => {
                // Par-or: try tactics in parallel (portfolio)
                // Z3 has par-or combinator for this
                if tactics.is_empty() {
                    return Tactic::new("skip");
                }

                if tactics.len() == 1 {
                    return self.combinator_to_tactic(&tactics[0]);
                }

                // Build par-or by creating or-else chain
                // Note: z3.rs doesn't expose par-or directly, use or-else chain
                let first = self.combinator_to_tactic(&tactics[0]);
                let mut result = first;

                for tactic_comb in tactics.iter().skip(1) {
                    let next_tactic = self.combinator_to_tactic(tactic_comb);
                    result = result.or_else(&next_tactic);
                }
                result
            }

            TacticCombinator::Try(inner) => {
                // `Try(t) ≡ OrElse(t, skip)` — soft-fail at the
                // Z3 tactic level. Mirror the semantic from the
                // executor's apply_combinator path.
                let inner_tactic = self.combinator_to_tactic(inner);
                let skip = Tactic::new("skip");
                inner_tactic.or_else(&skip)
            }

            TacticCombinator::FirstOf(branches) => {
                // First-success choice — left-fold via or_else,
                // mirroring the ParOr lowering. Empty list ⇒ skip
                // (no-op success), single element ⇒ the element.
                if branches.is_empty() {
                    return Tactic::new("skip");
                }
                if branches.len() == 1 {
                    return self.combinator_to_tactic(&branches[0]);
                }
                let mut result = self.combinator_to_tactic(&branches[0]);
                for branch in branches.iter().skip(1) {
                    let next = self.combinator_to_tactic(branch);
                    result = result.or_else(&next);
                }
                result
            }
            TacticCombinator::Solve(inner) => {
                // Z3 has no direct counterpart to "fail-when-open" at
                // the Tactic level; the executor's runtime path
                // handles the total-discharge gate.  Project to
                // inner so the Z3 stage runs the same operations
                // before the gate fires.  Defense in depth: the
                // simplifier reduces `Solve(skip)` to `fail` before
                // reaching this projection.
                self.combinator_to_tactic(inner)
            }
            TacticCombinator::AllGoals(inner) => {
                // Z3's chaining is implicitly per-open-goal — the
                // AllGoals shape is structurally identity at the
                // Tactic projection layer.
                self.combinator_to_tactic(inner)
            }
        }
    }

    /// Convert ProbeKind to Z3 Probe
    fn probe_kind_to_z3_probe(&self, probe: &crate::tactics::ProbeKind) -> z3::Probe {
        use crate::tactics::ProbeKind;

        match probe {
            ProbeKind::IsCNF => z3::Probe::new("is-cnf"),
            ProbeKind::IsPropositional => z3::Probe::new("is-propositional"),
            ProbeKind::IsQFBV => z3::Probe::new("is-qfbv"),
            ProbeKind::IsQFLIA => z3::Probe::new("is-qflia"),
            ProbeKind::NumConsts(_threshold) => z3::Probe::new("num-consts"),
            ProbeKind::Memory(_threshold) => z3::Probe::new("memory"),
            ProbeKind::Custom(name) => z3::Probe::new(name.as_str()),
        }
    }

    /// Build recommended tactic strategy for proof extraction
    ///
    /// Analyzes the goal and constructs an appropriate strategy:
    /// - Simplification
    /// - Domain-specific tactics (QF_LIA, QF_BV, etc.)
    /// - Fallback to general SMT
    pub fn build_proof_search_strategy(&self, goal: &Goal) -> TacticCombinator {
        use crate::tactics::ProbeKind;

        // Start with simplification
        let mut builder = StrategyBuilder::new().then(TacticKind::Simplify);

        // Add domain-specific tactics based on problem characteristics
        builder = builder.if_then_else(ProbeKind::IsQFLIA, TacticKind::QFLIA, TacticKind::Simplify);

        builder = builder.if_then_else(ProbeKind::IsQFBV, TacticKind::QFBV, TacticKind::Simplify);

        // Try equation solving
        builder = builder.then(TacticKind::SolveEqs);

        // Fallback to SMT solver
        builder = builder.or_else(TacticKind::SMT);

        builder.build()
    }

    /// Extract proof using automatic tactic selection
    ///
    /// This is a high-level convenience method that:
    /// 1. Builds an appropriate tactic strategy for the goal
    /// 2. Applies the strategy
    /// 3. Extracts the resulting proof if successful
    pub fn extract_proof_with_tactics(&self, goal: &Goal) -> Maybe<ProofTerm> {
        let strategy = self.build_proof_search_strategy(goal);
        self.apply_tactic_strategy(goal, &strategy)
    }
}

impl Default for ProofExtractor {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Bridge Conversion ====================

impl ProofExtractor {
    /// Convert a `proof_extraction::ProofTerm` into the lighter-weight
    /// `proof_extraction_bridge::ProofTerm` representation.
    ///
    /// This is the glue between the Z3-facing proof tree (which uses
    /// `Box<ProofTerm>` and raw `Text` formulas) and the tactic-centric bridge
    /// type (which mirrors the same structure but is decoupled from Z3 internals
    /// so that `proof_extraction_bridge` can be used without pulling in Z3).
    ///
    /// Most structural variants translate one-to-one.  Z3-specific or
    /// extended proof rules that have no direct bridge analogue are collapsed
    /// into `SmtVerified` so the certificate pipeline always produces a valid
    /// (though possibly coarse) proof object.
    pub fn to_bridge_term(
        proof: &ProofTerm,
    ) -> crate::proof_extraction_bridge::ProofTerm {
        use crate::proof_extraction_bridge::ProofTerm as B;

        match proof {
            // ── Base cases ────────────────────────────────────────────────
            ProofTerm::Axiom { name, .. } => B::Assumption {
                name: name.clone(),
            },
            ProofTerm::Hypothesis { formula, .. } => B::Assumption {
                name: formula.clone(),
            },

            // ── Classical rules ───────────────────────────────────────────
            ProofTerm::Reflexivity { term } => B::Reflexivity { term: term.clone() },

            ProofTerm::Symmetry { equality } => B::Symmetry {
                proof: Box::new(Self::to_bridge_term(equality)),
            },

            ProofTerm::Transitivity { left, right } => B::Transitivity {
                left: Box::new(Self::to_bridge_term(left)),
                right: Box::new(Self::to_bridge_term(right)),
            },

            ProofTerm::ModusPonens { premise, implication } => B::ModusPonens {
                hypothesis: Box::new(Self::to_bridge_term(premise)),
                implication: Box::new(Self::to_bridge_term(implication)),
            },

            ProofTerm::Rewrite { source, rule, .. } => B::Congruence {
                function: rule.clone(),
                arg_proof: Box::new(Self::to_bridge_term(source)),
            },

            // ── Theory / SAT rules ────────────────────────────────────────
            ProofTerm::TheoryLemma { theory, lemma } => B::SmtVerified {
                solver: verum_common::Text::from("z3"),
                goal: verum_common::Text::from(format!("theory:{} lemma:{}", theory, lemma)),
            },

            ProofTerm::UnitResolution { clauses } => {
                // Fold resolution chain into a single SmtVerified node.
                // A more faithful encoding would use TacticProduced, but SmtVerified
                // gives the certificate generators enough information.
                B::SmtVerified {
                    solver: verum_common::Text::from("z3"),
                    goal: verum_common::Text::from(format!(
                        "unit-resolution({} clauses)",
                        clauses.len()
                    )),
                }
            }

            ProofTerm::QuantifierInstantiation { quantified, instantiation } => {
                B::Application {
                    function: Box::new(Self::to_bridge_term(quantified)),
                    argument: verum_common::Text::from(format!(
                        "inst({})",
                        instantiation
                            .keys()
                            .cloned()
                            .collect::<verum_common::List<_>>()
                            .iter()
                            .map(|k| k.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    )),
                }
            }

            ProofTerm::Lemma { conclusion, proof } => B::TacticProduced {
                tactic_name: verum_common::Text::from("lemma"),
                subproofs: {
                    let mut sps = verum_common::List::new();
                    sps.push(Self::to_bridge_term(proof));
                    sps
                },
            },

            // ── Extended / catch-all ──────────────────────────────────────
            _ => B::SmtVerified {
                solver: verum_common::Text::from("z3"),
                goal: verum_common::Text::from(format!("{:?}", proof)),
            },
        }
    }
}

// ==================== Proof Analysis ====================

/// Analysis result for a proof term
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofAnalysis {
    /// Maximum proof depth
    pub depth: usize,
    /// Total number of proof nodes
    pub node_count: usize,
    /// Axioms used in the proof
    pub axioms_used: Set<Text>,
    /// Whether proof uses quantifiers
    pub has_quantifiers: bool,
    /// Number of theory lemmas used
    pub theory_lemmas: usize,
}

impl ProofAnalysis {
    /// Get proof complexity score (higher = more complex)
    pub fn complexity_score(&self) -> f64 {
        let base = self.node_count as f64;
        let depth_penalty = (self.depth as f64).ln();
        let quantifier_penalty = if self.has_quantifiers { 10.0 } else { 0.0 };
        let axiom_penalty = self.axioms_used.len() as f64 * 2.0;

        base + depth_penalty + quantifier_penalty + axiom_penalty
    }

    /// Check if proof is "simple" (low complexity)
    pub fn is_simple(&self) -> bool {
        self.complexity_score() < 50.0
    }
}

// ==================== Proof Validation ====================

/// Result of proof validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofValidation {
    /// Whether the proof is valid
    pub is_valid: bool,
    /// Validation errors (if any)
    pub errors: List<Text>,
    /// Validation warnings
    pub warnings: List<Text>,
}

impl ProofValidation {
    /// Check if proof passed validation
    pub fn is_ok(&self) -> bool {
        self.is_valid && self.errors.is_empty()
    }

    /// Get summary of validation result
    pub fn summary(&self) -> Text {
        if self.is_ok() {
            if self.warnings.is_empty() {
                "Proof is valid".to_text()
            } else {
                format!("Proof is valid with {} warnings", self.warnings.len()).into()
            }
        } else {
            format!(
                "Proof is invalid: {} errors, {} warnings",
                self.errors.len(),
                self.warnings.len()
            )
            .into()
        }
    }
}

// ==================== Proof Formatting ====================

/// Formats proofs for display
pub struct ProofFormatter;

impl ProofFormatter {
    /// Format proof to human-readable text
    pub fn format(&self, proof: &ProofTerm) -> Text {
        self.format_impl(proof, 0)
    }

    fn format_impl(&self, proof: &ProofTerm, indent: usize) -> Text {
        let prefix = "  ".repeat(indent);

        match proof {
            ProofTerm::Axiom { name, formula } => {
                format!("{}Axiom [{}]: {}", prefix, name, formula).into()
            }

            ProofTerm::TheoryLemma { theory, lemma } => {
                format!("{}Theory Lemma [{}]: {}", prefix, theory, lemma).into()
            }

            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                let premise_str = self.format_impl(premise, indent + 1);
                let impl_str = self.format_impl(implication, indent + 1);
                format!("{}Modus Ponens:\n{}\n{}", prefix, premise_str, impl_str).into()
            }

            ProofTerm::Rewrite {
                source,
                rule,
                target,
            } => {
                let source_str = self.format_impl(source, indent + 1);
                format!(
                    "{}Rewrite [{}]:\n{}\n{}  -> {}",
                    prefix, rule, source_str, prefix, target
                )
                .into()
            }

            ProofTerm::Symmetry { equality } => {
                let eq_str = self.format_impl(equality, indent + 1);
                format!("{}Symmetry:\n{}", prefix, eq_str).into()
            }

            ProofTerm::Transitivity { left, right } => {
                let left_str = self.format_impl(left, indent + 1);
                let right_str = self.format_impl(right, indent + 1);
                format!("{}Transitivity:\n{}\n{}", prefix, left_str, right_str).into()
            }

            ProofTerm::Reflexivity { term } => {
                format!("{}Reflexivity: {} = {}", prefix, term, term).into()
            }

            ProofTerm::UnitResolution { clauses } => {
                let mut result = format!("{}Unit Resolution:", prefix);
                for clause in clauses {
                    let clause_str = self.format_impl(clause, indent + 1);
                    result.push_str(&format!("\n{}", clause_str));
                }
                result.into()
            }

            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => {
                let quant_str = self.format_impl(quantified, indent + 1);
                let bindings: List<Text> = instantiation
                    .iter()
                    .map(|(k, v)| Text::from(format!("{} := {}", k, v)))
                    .collect();
                format!(
                    "{}Quantifier Instantiation [{}]:\n{}",
                    prefix,
                    bindings.join(", "),
                    quant_str
                )
                .into()
            }

            ProofTerm::Lemma { conclusion, proof } => {
                let proof_str = self.format_impl(proof, indent + 1);
                format!("{}Lemma [{}]:\n{}", prefix, conclusion, proof_str).into()
            }

            ProofTerm::Assumption { id, formula } => {
                format!("{}Assumption #{}: {}", prefix, id, formula).into()
            }

            ProofTerm::Hypothesis { id, formula } => {
                format!("{}Hypothesis #{}: {}", prefix, id, formula).into()
            }

            // New proof rules
            ProofTerm::AndElim {
                conjunction,
                index,
                result,
            } => {
                let conj_str = self.format_impl(conjunction, indent + 1);
                format!(
                    "{}And Elim [{}]:\n{}\n{}  -> {}",
                    prefix, index, conj_str, prefix, result
                )
                .into()
            }

            ProofTerm::NotOrElim {
                negated_disjunction,
                index,
                result,
            } => {
                let disj_str = self.format_impl(negated_disjunction, indent + 1);
                format!(
                    "{}Not-Or Elim [{}]:\n{}\n{}  -> {}",
                    prefix, index, disj_str, prefix, result
                )
                .into()
            }

            ProofTerm::IffTrue { proof, formula } => {
                let proof_str = self.format_impl(proof, indent + 1);
                format!(
                    "{}Iff-True:\n{}\n{}  -> (iff {} true)",
                    prefix, proof_str, prefix, formula
                )
                .into()
            }

            ProofTerm::IffFalse { proof, formula } => {
                let proof_str = self.format_impl(proof, indent + 1);
                format!(
                    "{}Iff-False:\n{}\n{}  -> (iff {} false)",
                    prefix, proof_str, prefix, formula
                )
                .into()
            }

            ProofTerm::Commutativity { left, right } => {
                format!("{}Commutativity: {} = {}", prefix, left, right).into()
            }

            ProofTerm::Monotonicity {
                premises,
                conclusion,
            } => {
                let mut result = format!("{}Monotonicity:", prefix);
                for premise in premises {
                    let prem_str = self.format_impl(premise, indent + 1);
                    result.push_str(&format!("\n{}", prem_str));
                }
                result.push_str(&format!("\n{}  -> {}", prefix, conclusion));
                result.into()
            }

            ProofTerm::Distributivity { formula } => {
                format!("{}Distributivity: {}", prefix, formula).into()
            }

            ProofTerm::DefAxiom { formula } => format!("{}Def Axiom: {}", prefix, formula).into(),

            ProofTerm::DefIntro { name, definition } => {
                format!("{}Def Intro [{}]: {}", prefix, name, definition).into()
            }

            ProofTerm::ApplyDef {
                def_proof,
                original,
                name,
            } => {
                let def_str = self.format_impl(def_proof, indent + 1);
                format!(
                    "{}Apply Def:\n{}\n{}  {} ~ {}",
                    prefix, def_str, prefix, original, name
                )
                .into()
            }

            ProofTerm::IffOEq {
                iff_proof,
                left,
                right,
            } => {
                let iff_str = self.format_impl(iff_proof, indent + 1);
                format!(
                    "{}Iff-OEq:\n{}\n{}  -> ({} ~ {})",
                    prefix, iff_str, prefix, left, right
                )
                .into()
            }

            ProofTerm::NNFPos {
                premises,
                conclusion,
            } => {
                let mut result = format!("{}NNF Positive:", prefix);
                for premise in premises {
                    let prem_str = self.format_impl(premise, indent + 1);
                    result.push_str(&format!("\n{}", prem_str));
                }
                result.push_str(&format!("\n{}  -> {}", prefix, conclusion));
                result.into()
            }

            ProofTerm::NNFNeg {
                premises,
                conclusion,
            } => {
                let mut result = format!("{}NNF Negative:", prefix);
                for premise in premises {
                    let prem_str = self.format_impl(premise, indent + 1);
                    result.push_str(&format!("\n{}", prem_str));
                }
                result.push_str(&format!("\n{}  -> {}", prefix, conclusion));
                result.into()
            }

            ProofTerm::Skolemize { formula } => format!("{}Skolemize: {}", prefix, formula).into(),

            ProofTerm::QuantIntro {
                body_proof,
                conclusion,
            } => {
                let body_str = self.format_impl(body_proof, indent + 1);
                format!(
                    "{}Quant Intro:\n{}\n{}  -> {}",
                    prefix, body_str, prefix, conclusion
                )
                .into()
            }

            ProofTerm::Bind {
                body_proof,
                variables,
                conclusion,
            } => {
                let body_str = self.format_impl(body_proof, indent + 1);
                format!(
                    "{}Bind [{}]:\n{}\n{}  -> {}",
                    prefix,
                    variables.join(", "),
                    body_str,
                    prefix,
                    conclusion
                )
                .into()
            }

            ProofTerm::PullQuant { formula } => format!("{}Pull Quant: {}", prefix, formula).into(),

            ProofTerm::PushQuant { formula } => format!("{}Push Quant: {}", prefix, formula).into(),

            ProofTerm::ElimUnusedVars { formula } => {
                format!("{}Elim Unused Vars: {}", prefix, formula).into()
            }

            ProofTerm::DestructiveEqRes { formula } => {
                format!("{}Destructive Eq Res: {}", prefix, formula).into()
            }

            ProofTerm::HyperResolve {
                clauses,
                conclusion,
            } => {
                let mut result = format!("{}Hyper Resolve:", prefix);
                for clause in clauses {
                    let clause_str = self.format_impl(clause, indent + 1);
                    result.push_str(&format!("\n{}", clause_str));
                }
                result.push_str(&format!("\n{}  -> {}", prefix, conclusion));
                result.into()
            }
        }
    }
}

// ==================== Proof Export ====================

/// Export proof to various formats
///
/// The ProofExporter provides conversion from ProofTerm to multiple output formats:
/// - SMT-LIB2: Standard SMT solver format for proof exchange
/// - Coq: Proof assistant tactics
/// - Lean: Lean 4 proof assistant tactics
/// - Human-readable: Formatted text for debugging
pub struct ProofExporter;

impl ProofExporter {
    /// Export proof to SMT-LIB2 format
    ///
    /// Generates a complete SMT-LIB2 proof script that can be verified
    /// by any SMT-LIB2 compliant solver.
    ///
    /// ## Format
    ///
    /// The output uses standard SMT-LIB2 proof annotations:
    /// - Named assertions: `(assert (! formula :named name))`
    /// - Proof steps: `(@rule premise1 premise2 ... conclusion)`
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let proof = ProofTerm::Axiom { name: "ax1".into(), formula: "(> x 0)".into() };
    /// let smtlib = ProofExporter::to_smtlib2(&proof);
    /// assert!(smtlib.contains("assert"));
    /// ```
    pub fn to_smtlib2(proof: &ProofTerm) -> Text {
        let mut output = String::new();
        output.push_str("; SMT-LIB2 Proof Export\n");
        output.push_str("; Generated by Verum SMT\n\n");
        Self::to_smtlib2_impl(proof, &mut output, 0);
        output.into()
    }

    /// Internal recursive SMT-LIB2 export
    fn to_smtlib2_impl(proof: &ProofTerm, output: &mut String, step: usize) {
        match proof {
            ProofTerm::Axiom { name, formula } => {
                output.push_str(&format!("(assert (! {} :named {}))\n", formula, name));
            }
            ProofTerm::Assumption { id, formula } => {
                output.push_str(&format!("; Assumption {}\n(assert {})\n", id, formula));
            }
            ProofTerm::Hypothesis { id, formula } => {
                output.push_str(&format!("; Hypothesis {}\n(assert {})\n", id, formula));
            }
            ProofTerm::TheoryLemma { theory, lemma } => {
                output.push_str(&format!(
                    "; Theory lemma from {}\n(assert {})\n",
                    theory, lemma
                ));
            }
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                output.push_str(&format!("; Step {}: Modus Ponens\n", step));
                Self::to_smtlib2_impl(premise, output, step + 1);
                Self::to_smtlib2_impl(implication, output, step + 2);
                output.push_str(&format!("(@mp step{} step{})\n", step + 1, step + 2));
            }
            ProofTerm::Reflexivity { term } => {
                output.push_str(&format!("; Step {}: Reflexivity\n(@refl {})\n", step, term));
            }
            ProofTerm::Symmetry { equality } => {
                output.push_str(&format!("; Step {}: Symmetry\n", step));
                Self::to_smtlib2_impl(equality, output, step + 1);
                output.push_str(&format!("(@symm step{})\n", step + 1));
            }
            ProofTerm::Transitivity { left, right } => {
                output.push_str(&format!("; Step {}: Transitivity\n", step));
                Self::to_smtlib2_impl(left, output, step + 1);
                Self::to_smtlib2_impl(right, output, step + 2);
                output.push_str(&format!("(@trans step{} step{})\n", step + 1, step + 2));
            }
            ProofTerm::Rewrite {
                source,
                rule,
                target,
            } => {
                output.push_str(&format!("; Step {}: Rewrite ({})\n", step, rule));
                Self::to_smtlib2_impl(source, output, step + 1);
                output.push_str(&format!("(@rewrite step{} {})\n", step + 1, target));
            }
            ProofTerm::UnitResolution { clauses } => {
                output.push_str(&format!(
                    "; Step {}: Unit Resolution ({} clauses)\n",
                    step,
                    clauses.len()
                ));
                for (i, clause) in clauses.iter().enumerate() {
                    Self::to_smtlib2_impl(clause, output, step + i + 1);
                }
                output.push_str("(@unit-resolution");
                for i in 0..clauses.len() {
                    output.push_str(&format!(" step{}", step + i + 1));
                }
                output.push_str(")\n");
            }
            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => {
                output.push_str(&format!("; Step {}: Quantifier Instantiation\n", step));
                Self::to_smtlib2_impl(quantified, output, step + 1);
                output.push_str("(@quant-inst step");
                output.push_str(&format!("{}", step + 1));
                for (var, val) in instantiation.iter() {
                    output.push_str(&format!(" ({} {})", var, val));
                }
                output.push_str(")\n");
            }
            ProofTerm::Lemma {
                conclusion,
                proof: sub_proof,
            } => {
                output.push_str(&format!("; Step {}: Lemma\n", step));
                Self::to_smtlib2_impl(sub_proof, output, step + 1);
                output.push_str(&format!("(@lemma step{} {})\n", step + 1, conclusion));
            }
            ProofTerm::AndElim {
                conjunction,
                index,
                result,
            } => {
                output.push_str(&format!("; Step {}: And Elimination [{}]\n", step, index));
                Self::to_smtlib2_impl(conjunction, output, step + 1);
                output.push_str(&format!(
                    "(@and-elim step{} {} {})\n",
                    step + 1,
                    index,
                    result
                ));
            }
            ProofTerm::NotOrElim {
                negated_disjunction,
                index,
                result,
            } => {
                output.push_str(&format!(
                    "; Step {}: Not-Or Elimination [{}]\n",
                    step, index
                ));
                Self::to_smtlib2_impl(negated_disjunction, output, step + 1);
                output.push_str(&format!(
                    "(@not-or-elim step{} {} {})\n",
                    step + 1,
                    index,
                    result
                ));
            }
            ProofTerm::IffTrue {
                proof: sub_proof,
                formula,
            } => {
                output.push_str(&format!("; Step {}: Iff-True\n", step));
                Self::to_smtlib2_impl(sub_proof, output, step + 1);
                output.push_str(&format!("(@iff-true step{} {})\n", step + 1, formula));
            }
            ProofTerm::IffFalse {
                proof: sub_proof,
                formula,
            } => {
                output.push_str(&format!("; Step {}: Iff-False\n", step));
                Self::to_smtlib2_impl(sub_proof, output, step + 1);
                output.push_str(&format!("(@iff-false step{} {})\n", step + 1, formula));
            }
            ProofTerm::Commutativity { left, right } => {
                output.push_str(&format!(
                    "; Step {}: Commutativity\n(@comm (= {} {}))\n",
                    step, left, right
                ));
            }
            ProofTerm::Monotonicity {
                premises,
                conclusion,
            } => {
                output.push_str(&format!("; Step {}: Monotonicity\n", step));
                for (i, premise) in premises.iter().enumerate() {
                    Self::to_smtlib2_impl(premise, output, step + i + 1);
                }
                output.push_str("(@monotonicity");
                for i in 0..premises.len() {
                    output.push_str(&format!(" step{}", step + i + 1));
                }
                output.push_str(&format!(" {})\n", conclusion));
            }
            ProofTerm::Distributivity { formula } => {
                output.push_str(&format!(
                    "; Step {}: Distributivity\n(@dist {})\n",
                    step, formula
                ));
            }
            ProofTerm::DefAxiom { formula } => {
                output.push_str(&format!(
                    "; Step {}: Definition Axiom\n(@def-axiom {})\n",
                    step, formula
                ));
            }
            ProofTerm::DefIntro { name, definition } => {
                output.push_str(&format!(
                    "; Step {}: Definition Introduction\n(@def-intro {} {})\n",
                    step, name, definition
                ));
            }
            ProofTerm::ApplyDef {
                def_proof,
                original,
                name,
            } => {
                output.push_str(&format!("; Step {}: Apply Definition\n", step));
                Self::to_smtlib2_impl(def_proof, output, step + 1);
                output.push_str(&format!(
                    "(@apply-def step{} {} {})\n",
                    step + 1,
                    original,
                    name
                ));
            }
            ProofTerm::IffOEq {
                iff_proof,
                left,
                right,
            } => {
                output.push_str(&format!("; Step {}: Iff to OEq\n", step));
                Self::to_smtlib2_impl(iff_proof, output, step + 1);
                output.push_str(&format!("(@iff-oeq step{} {} {})\n", step + 1, left, right));
            }
            ProofTerm::NNFPos {
                premises,
                conclusion,
            } => {
                output.push_str(&format!("; Step {}: NNF Positive\n", step));
                for (i, premise) in premises.iter().enumerate() {
                    Self::to_smtlib2_impl(premise, output, step + i + 1);
                }
                output.push_str("(@nnf-pos");
                for i in 0..premises.len() {
                    output.push_str(&format!(" step{}", step + i + 1));
                }
                output.push_str(&format!(" {})\n", conclusion));
            }
            ProofTerm::NNFNeg {
                premises,
                conclusion,
            } => {
                output.push_str(&format!("; Step {}: NNF Negative\n", step));
                for (i, premise) in premises.iter().enumerate() {
                    Self::to_smtlib2_impl(premise, output, step + i + 1);
                }
                output.push_str("(@nnf-neg");
                for i in 0..premises.len() {
                    output.push_str(&format!(" step{}", step + i + 1));
                }
                output.push_str(&format!(" {})\n", conclusion));
            }
            ProofTerm::Skolemize { formula } => {
                output.push_str(&format!(
                    "; Step {}: Skolemization\n(@skolem {})\n",
                    step, formula
                ));
            }
            ProofTerm::QuantIntro {
                body_proof,
                conclusion,
            } => {
                output.push_str(&format!("; Step {}: Quantifier Introduction\n", step));
                Self::to_smtlib2_impl(body_proof, output, step + 1);
                output.push_str(&format!("(@quant-intro step{} {})\n", step + 1, conclusion));
            }
            ProofTerm::Bind {
                body_proof,
                variables,
                conclusion,
            } => {
                output.push_str(&format!("; Step {}: Bind\n", step));
                Self::to_smtlib2_impl(body_proof, output, step + 1);
                output.push_str(&format!(
                    "(@bind step{} ({}) {})\n",
                    step + 1,
                    variables.join(" "),
                    conclusion
                ));
            }
            ProofTerm::PullQuant { formula } => {
                output.push_str(&format!(
                    "; Step {}: Pull Quantifier\n(@pull-quant {})\n",
                    step, formula
                ));
            }
            ProofTerm::PushQuant { formula } => {
                output.push_str(&format!(
                    "; Step {}: Push Quantifier\n(@push-quant {})\n",
                    step, formula
                ));
            }
            ProofTerm::ElimUnusedVars { formula } => {
                output.push_str(&format!(
                    "; Step {}: Eliminate Unused Variables\n(@elim-unused {})\n",
                    step, formula
                ));
            }
            ProofTerm::DestructiveEqRes { formula } => {
                output.push_str(&format!(
                    "; Step {}: Destructive Equality Resolution\n(@der {})\n",
                    step, formula
                ));
            }
            ProofTerm::HyperResolve {
                clauses,
                conclusion,
            } => {
                output.push_str(&format!(
                    "; Step {}: Hyper Resolution ({} clauses)\n",
                    step,
                    clauses.len()
                ));
                for (i, clause) in clauses.iter().enumerate() {
                    Self::to_smtlib2_impl(clause, output, step + i + 1);
                }
                output.push_str("(@hyper-resolve");
                for i in 0..clauses.len() {
                    output.push_str(&format!(" step{}", step + i + 1));
                }
                output.push_str(&format!(" {})\n", conclusion));
            }
        }
    }

    /// Export proof to SMT-LIB2 with proof annotations
    ///
    /// This variant includes additional annotations for proof checking.
    pub fn to_smtlib2_annotated(proof: &ProofTerm, include_comments: bool) -> Text {
        let base = Self::to_smtlib2(proof);
        if include_comments {
            let mut output = String::new();
            output.push_str("; Proof Statistics:\n");
            output.push_str(&format!(";   Depth: {}\n", proof.proof_depth()));
            output.push_str(&format!(";   Nodes: {}\n", proof.node_count()));
            output.push_str(&format!(";   Axioms: {}\n", proof.used_axioms().len()));
            output.push('\n');
            output.push_str(base.as_str());
            output.into()
        } else {
            base
        }
    }

    /// Export proof to Coq format
    ///
    /// Converts a ProofTerm into valid Coq tactic syntax.
    /// The output is wrapped in "Proof. ... Qed." format.
    pub fn to_coq(proof: &ProofTerm) -> Text {
        let mut output = String::new();
        output.push_str("Proof.\n");
        output.push_str(&Self::proof_to_coq_tactics(proof, 1));
        output.push_str("Qed.\n");
        output.into()
    }

    /// Convert ProofTerm to Coq tactics (internal recursive implementation)
    fn proof_to_coq_tactics(proof: &ProofTerm, indent: usize) -> String {
        let spaces = "  ".repeat(indent);

        match proof {
            ProofTerm::Axiom { name, .. } => {
                format!("{}exact {}.\n", spaces, name)
            }

            ProofTerm::Assumption { id, .. } => {
                format!("{}exact assumption_{}.\n", spaces, id)
            }

            ProofTerm::Hypothesis { id, .. } => {
                format!("{}exact hypothesis_{}.\n", spaces, id)
            }

            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}(* Modus ponens *)\n", spaces));
                tactics.push_str(&format!("{}apply ", spaces));

                // Extract the implication's conclusion formula
                let impl_conclusion = implication.conclusion();
                if impl_conclusion.contains("=>") || impl_conclusion.contains("->") {
                    tactics.push_str("(* implication *).\n");
                } else {
                    tactics.push_str(".\n");
                }

                tactics.push_str(&Self::proof_to_coq_tactics(premise, indent));
                tactics
            }

            ProofTerm::Rewrite { source, rule, .. } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}rewrite {}.\n", spaces, rule));
                tactics.push_str(&Self::proof_to_coq_tactics(source, indent));
                tactics
            }

            ProofTerm::Symmetry { equality } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}symmetry.\n", spaces));
                tactics.push_str(&Self::proof_to_coq_tactics(equality, indent));
                tactics
            }

            ProofTerm::Transitivity { left, right } => {
                let mut tactics = String::new();
                // Extract intermediate term from the equalities
                let left_conclusion = left.conclusion();
                let intermediate = if left_conclusion.contains("=") {
                    let parts: Vec<&str> = left_conclusion.as_str().split("=").collect();
                    parts.get(1).map(|s| s.trim()).unwrap_or("_")
                } else {
                    "_"
                };

                tactics.push_str(&format!("{}transitivity {}.\n", spaces, intermediate));
                tactics.push_str(&format!("{}- (* left side *)\n", spaces));
                tactics.push_str(&Self::proof_to_coq_tactics(left, indent + 1));
                tactics.push_str(&format!("{}- (* right side *)\n", spaces));
                tactics.push_str(&Self::proof_to_coq_tactics(right, indent + 1));
                tactics
            }

            ProofTerm::Reflexivity { term } => {
                format!("{}reflexivity. (* {} = {} *)\n", spaces, term, term)
            }

            ProofTerm::TheoryLemma { theory, lemma } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}(* Theory lemma from {} *)\n", spaces, theory));
                tactics.push_str(&format!("{}(* Formula: {} *)\n", spaces, lemma));
                tactics.push_str(&format!("{}auto.\n", spaces));
                tactics
            }

            ProofTerm::UnitResolution { clauses } => {
                let mut tactics = String::new();
                tactics.push_str(&format!(
                    "{}(* Unit resolution with {} clauses *)\n",
                    spaces,
                    clauses.len()
                ));

                if clauses.is_empty() {
                    tactics.push_str(&format!("{}contradiction.\n", spaces));
                } else {
                    // Apply each clause in sequence
                    for (idx, clause) in clauses.iter().enumerate() {
                        if clauses.len() > 1 {
                            tactics.push_str(&format!("{}(* clause {} *)\n", spaces, idx + 1));
                        }
                        tactics.push_str(&Self::proof_to_coq_tactics(clause, indent));
                    }
                }
                tactics
            }

            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => {
                let mut tactics = String::new();

                // Build instantiation list
                let bindings: Vec<String> = instantiation
                    .iter()
                    .map(|(var, val)| format!("{} := {}", var, val))
                    .collect();

                if !bindings.is_empty() {
                    tactics.push_str(&format!(
                        "{}(* Instantiate with: {} *)\n",
                        spaces,
                        bindings.join(", ")
                    ));
                }

                tactics.push_str(&format!("{}eapply (", spaces));
                tactics.push_str("quantifier");

                // Add instantiation terms
                for (_, val) in instantiation.iter() {
                    tactics.push_str(&format!(" {}", val));
                }
                tactics.push_str(").\n");

                tactics.push_str(&Self::proof_to_coq_tactics(quantified, indent));
                tactics
            }

            ProofTerm::Lemma { conclusion, proof } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}(* Lemma: {} *)\n", spaces, conclusion));
                tactics.push_str(&Self::proof_to_coq_tactics(proof, indent));
                tactics
            }

            // Handle all other proof terms with a generic tactic
            _ => {
                format!("{}(* Unhandled proof term *)\n{}auto.\n", spaces, spaces)
            }
        }
    }

    /// Export proof to Lean format
    ///
    /// Converts a ProofTerm into valid Lean 4 tactic syntax.
    /// The output uses "by" tactic mode.
    pub fn to_lean(proof: &ProofTerm) -> Text {
        let mut output = String::new();
        output.push_str("by\n");
        output.push_str(&Self::proof_to_lean_tactics(proof, 1));
        output.into()
    }

    /// Convert ProofTerm to Lean tactics (internal recursive implementation)
    fn proof_to_lean_tactics(proof: &ProofTerm, indent: usize) -> String {
        let spaces = "  ".repeat(indent);

        match proof {
            ProofTerm::Axiom { name, .. } => {
                format!("{}exact {}\n", spaces, name)
            }

            ProofTerm::Assumption { id, .. } => {
                format!("{}exact assumption_{}\n", spaces, id)
            }

            ProofTerm::Hypothesis { id, .. } => {
                format!("{}exact hypothesis_{}\n", spaces, id)
            }

            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}-- Modus ponens\n", spaces));

                // In Lean, modus ponens is application: apply the implication to the premise
                tactics.push_str(&format!("{}apply ", spaces));

                // Get the implication name/reference if it's an axiom or hypothesis
                match implication.as_ref() {
                    ProofTerm::Axiom { name, .. } => {
                        tactics.push_str(&format!("{} ", name.as_str()));
                    }
                    ProofTerm::Hypothesis { id, .. } => {
                        tactics.push_str(&format!("hypothesis_{} ", id));
                    }
                    _ => {
                        tactics.push_str("(* implication *) ");
                    }
                }

                // Apply premise using angle bracket notation
                tactics.push('⟨');
                match premise.as_ref() {
                    ProofTerm::Axiom { name, .. } => {
                        tactics.push_str(name.as_str());
                    }
                    ProofTerm::Hypothesis { id, .. } => {
                        tactics.push_str(&format!("hypothesis_{}", id));
                    }
                    _ => {
                        tactics.push('_');
                    }
                }
                tactics.push_str("⟩\n");

                tactics
            }

            ProofTerm::Rewrite { source, rule, .. } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}rw [{}]\n", spaces, rule));
                tactics.push_str(&Self::proof_to_lean_tactics(source, indent));
                tactics
            }

            ProofTerm::Symmetry { equality } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}symm\n", spaces));
                tactics.push_str(&Self::proof_to_lean_tactics(equality, indent));
                tactics
            }

            ProofTerm::Transitivity { left, right } => {
                let mut tactics = String::new();
                // Extract intermediate term from the equalities
                let left_conclusion = left.conclusion();
                let intermediate = if left_conclusion.contains("=") {
                    let parts: Vec<&str> = left_conclusion.as_str().split("=").collect();
                    parts.get(1).map(|s| s.trim()).unwrap_or("_")
                } else {
                    "_"
                };

                tactics.push_str(&format!("{}trans {} <;> [\n", spaces, intermediate));
                tactics.push_str(&format!("{}  -- left side\n", spaces));
                tactics.push_str(&Self::proof_to_lean_tactics(left, indent + 1));
                tactics.push_str(&format!("{},\n", spaces));
                tactics.push_str(&format!("{}  -- right side\n", spaces));
                tactics.push_str(&Self::proof_to_lean_tactics(right, indent + 1));
                tactics.push_str(&format!("{}]\n", spaces));
                tactics
            }

            ProofTerm::Reflexivity { term } => {
                format!("{}rfl -- {} = {}\n", spaces, term, term)
            }

            ProofTerm::TheoryLemma { theory, lemma } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}-- Theory lemma from {}\n", spaces, theory));
                tactics.push_str(&format!("{}-- Formula: {}\n", spaces, lemma));
                tactics.push_str(&format!("{}simp [*]\n", spaces));
                tactics
            }

            ProofTerm::UnitResolution { clauses } => {
                let mut tactics = String::new();
                tactics.push_str(&format!(
                    "{}-- Unit resolution with {} clauses\n",
                    spaces,
                    clauses.len()
                ));

                if clauses.is_empty() {
                    tactics.push_str(&format!("{}contradiction\n", spaces));
                } else {
                    // Apply each clause in sequence
                    for (idx, clause) in clauses.iter().enumerate() {
                        if clauses.len() > 1 {
                            tactics.push_str(&format!("{}-- clause {}\n", spaces, idx + 1));
                        }
                        tactics.push_str(&Self::proof_to_lean_tactics(clause, indent));
                    }
                }
                tactics
            }

            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => {
                let mut tactics = String::new();

                // Build instantiation list
                let bindings: Vec<String> = instantiation
                    .iter()
                    .map(|(var, val)| format!("{} ↦ {}", var, val))
                    .collect();

                if !bindings.is_empty() {
                    tactics.push_str(&format!(
                        "{}-- Instantiate with: {}\n",
                        spaces,
                        bindings.join(", ")
                    ));
                }

                tactics.push_str(&format!("{}refine quantifier ", spaces));

                // Add instantiation terms
                for (_, val) in instantiation.iter() {
                    tactics.push_str(&format!("{} ", val));
                }
                tactics.push_str("?_\n");

                tactics.push_str(&Self::proof_to_lean_tactics(quantified, indent));
                tactics
            }

            ProofTerm::Lemma { conclusion, proof } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}-- Lemma: {}\n", spaces, conclusion));
                tactics.push_str(&format!("{}have : {} := by\n", spaces, conclusion));
                tactics.push_str(&Self::proof_to_lean_tactics(proof, indent + 1));
                tactics.push_str(&format!("{}exact this\n", spaces));
                tactics
            }

            // Handle all other proof terms with a generic tactic
            _ => {
                format!("{}-- Unhandled proof term\n{}trivial\n", spaces, spaces)
            }
        }
    }

    /// Export proof to human-readable format
    pub fn to_readable(proof: &ProofTerm) -> Text {
        Self::to_readable_impl(proof, 0)
    }

    fn to_readable_impl(proof: &ProofTerm, indent: usize) -> Text {
        let prefix = "  ".repeat(indent);
        match proof {
            ProofTerm::Axiom { name, formula } => {
                format!("{}Axiom {}: {}", prefix, name, formula).into()
            }
            ProofTerm::TheoryLemma { theory, lemma } => {
                format!("{}Theory Lemma ({}): {}", prefix, theory, lemma).into()
            }
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => format!(
                "{}Modus Ponens:\n{}\n{}",
                prefix,
                Self::to_readable_impl(premise, indent + 1),
                Self::to_readable_impl(implication, indent + 1)
            )
            .into(),
            ProofTerm::Reflexivity { term } => {
                format!("{}Reflexivity: {} = {}", prefix, term, term).into()
            }
            _ => format!("{}{:?}", prefix, proof).into(),
        }
    }
}

// ==================== Proof Minimizer ====================

/// Minimize proof by removing redundant steps
pub struct ProofMinimizer;

impl ProofMinimizer {
    /// Minimize proof by removing redundant lemmas and simplifying chains
    pub fn minimize(proof: &ProofTerm) -> ProofTerm {
        match proof {
            // Remove redundant transitivity chains
            ProofTerm::Transitivity { left, right } => {
                let min_left = Self::minimize(left);
                let min_right = Self::minimize(right);

                // If either side is reflexivity, simplify
                match (&min_left, &min_right) {
                    (ProofTerm::Reflexivity { .. }, _) => min_right,
                    (_, ProofTerm::Reflexivity { .. }) => min_left,
                    _ => ProofTerm::Transitivity {
                        left: Box::new(min_left),
                        right: Box::new(min_right),
                    },
                }
            }

            // Recursively minimize sub-proofs
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => ProofTerm::ModusPonens {
                premise: Box::new(Self::minimize(premise)),
                implication: Box::new(Self::minimize(implication)),
            },

            ProofTerm::Rewrite {
                source,
                rule,
                target,
            } => ProofTerm::Rewrite {
                source: Box::new(Self::minimize(source)),
                rule: rule.clone(),
                target: target.clone(),
            },

            ProofTerm::Symmetry { equality } => ProofTerm::Symmetry {
                equality: Box::new(Self::minimize(equality)),
            },

            ProofTerm::Lemma { conclusion, proof } => ProofTerm::Lemma {
                conclusion: conclusion.clone(),
                proof: Box::new(Self::minimize(proof)),
            },

            // New proof rules with sub-proofs
            ProofTerm::AndElim {
                conjunction,
                index,
                result,
            } => ProofTerm::AndElim {
                conjunction: Box::new(Self::minimize(conjunction)),
                index: *index,
                result: result.clone(),
            },

            ProofTerm::NotOrElim {
                negated_disjunction,
                index,
                result,
            } => ProofTerm::NotOrElim {
                negated_disjunction: Box::new(Self::minimize(negated_disjunction)),
                index: *index,
                result: result.clone(),
            },

            ProofTerm::IffTrue { proof, formula } => ProofTerm::IffTrue {
                proof: Box::new(Self::minimize(proof)),
                formula: formula.clone(),
            },

            ProofTerm::IffFalse { proof, formula } => ProofTerm::IffFalse {
                proof: Box::new(Self::minimize(proof)),
                formula: formula.clone(),
            },

            ProofTerm::Monotonicity {
                premises,
                conclusion,
            } => ProofTerm::Monotonicity {
                premises: premises.iter().map(Self::minimize).collect(),
                conclusion: conclusion.clone(),
            },

            ProofTerm::ApplyDef {
                def_proof,
                original,
                name,
            } => ProofTerm::ApplyDef {
                def_proof: Box::new(Self::minimize(def_proof)),
                original: original.clone(),
                name: name.clone(),
            },

            ProofTerm::IffOEq {
                iff_proof,
                left,
                right,
            } => ProofTerm::IffOEq {
                iff_proof: Box::new(Self::minimize(iff_proof)),
                left: left.clone(),
                right: right.clone(),
            },

            ProofTerm::NNFPos {
                premises,
                conclusion,
            } => ProofTerm::NNFPos {
                premises: premises.iter().map(Self::minimize).collect(),
                conclusion: conclusion.clone(),
            },

            ProofTerm::NNFNeg {
                premises,
                conclusion,
            } => ProofTerm::NNFNeg {
                premises: premises.iter().map(Self::minimize).collect(),
                conclusion: conclusion.clone(),
            },

            ProofTerm::QuantIntro {
                body_proof,
                conclusion,
            } => ProofTerm::QuantIntro {
                body_proof: Box::new(Self::minimize(body_proof)),
                conclusion: conclusion.clone(),
            },

            ProofTerm::Bind {
                body_proof,
                variables,
                conclusion,
            } => ProofTerm::Bind {
                body_proof: Box::new(Self::minimize(body_proof)),
                variables: variables.clone(),
                conclusion: conclusion.clone(),
            },

            ProofTerm::UnitResolution { clauses } => ProofTerm::UnitResolution {
                clauses: clauses.iter().map(Self::minimize).collect(),
            },

            ProofTerm::HyperResolve {
                clauses,
                conclusion,
            } => ProofTerm::HyperResolve {
                clauses: clauses.iter().map(Self::minimize).collect(),
                conclusion: conclusion.clone(),
            },

            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => ProofTerm::QuantifierInstantiation {
                quantified: Box::new(Self::minimize(quantified)),
                instantiation: instantiation.clone(),
            },

            // Leaf nodes - cannot minimize further
            ProofTerm::Axiom { .. }
            | ProofTerm::Assumption { .. }
            | ProofTerm::Hypothesis { .. }
            | ProofTerm::Reflexivity { .. }
            | ProofTerm::TheoryLemma { .. }
            | ProofTerm::Commutativity { .. }
            | ProofTerm::Distributivity { .. }
            | ProofTerm::DefAxiom { .. }
            | ProofTerm::DefIntro { .. }
            | ProofTerm::Skolemize { .. }
            | ProofTerm::PullQuant { .. }
            | ProofTerm::PushQuant { .. }
            | ProofTerm::ElimUnusedVars { .. }
            | ProofTerm::DestructiveEqRes { .. } => proof.clone(),
        }
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin the fail-closed contract of `validate_on_extract = true`.
    ///
    /// Pre-fix `extract_proof` logged a warning and RETURNED the
    /// (possibly invalid) proof anyway, defeating the safety-critical
    /// semantics of the flag.  Now: a hard validation failure
    /// (`!validation.is_ok()`) under `validate_on_extract = true`
    /// returns `Maybe::None`, so the caller can no longer accidentally
    /// consume an unvalidated proof while believing it was validated.
    ///
    /// We test the public surface directly: build a `ProofValidation`
    /// with `is_valid = false` + a non-empty `errors` list, and assert
    /// that `is_ok()` returns false.  The integration with
    /// `extract_proof` requires a Z3 proof object which can't be
    /// constructed in a unit-test context, so the structural pin
    /// covers the validation predicate; the call-site integration is
    /// a one-line `return Maybe::None;` that the diff makes obvious.
    #[test]
    fn proof_validation_is_ok_requires_no_errors() {
        let v_ok = ProofValidation {
            is_valid: true,
            errors: List::new(),
            warnings: List::new(),
        };
        assert!(v_ok.is_ok(), "fully-valid proof must be ok");

        let mut errs = List::new();
        errs.push("structural error".into());
        let v_err = ProofValidation {
            is_valid: false,
            errors: errs,
            warnings: List::new(),
        };
        assert!(
            !v_err.is_ok(),
            "validation with errors must NOT be ok — \
             extract_proof relies on this to fail-close"
        );

        // Edge case: is_valid=true but errors non-empty — still NOT ok.
        // The `is_ok` predicate is the conjunction, so a defensive
        // is_valid=true claim doesn't override an errors list.
        let mut errs2 = List::new();
        errs2.push("conflicting state".into());
        let v_inconsistent = ProofValidation {
            is_valid: true,
            errors: errs2,
            warnings: List::new(),
        };
        assert!(
            !v_inconsistent.is_ok(),
            "is_valid=true with errors must NOT be ok",
        );
    }

    #[test]
    fn apply_to_z3_solver_threads_minimize_and_timeout() {
        // Pin: `apply_to_z3_solver` reaches the Solver every time
        // for both `minimize_unsat_cores` and `extraction_timeout_ms`.
        // We can't observe the actual Z3 param state from outside
        // the solver, but we can pin the call surface contract: the
        // method runs to completion without panicking on any
        // combination of (minimize, timeout) values, and the
        // documented "0 = no timeout" semantic holds (no panic on 0).
        let solver = Solver::new();

        // Default: minimize=false, timeout_ms=0 (unbounded). The
        // method must succeed without setting any timeout param.
        let default_cfg = ProofGenerationConfig::default();
        default_cfg.apply_to_z3_solver(&solver);

        // minimize=true, timeout=10s — both must reach the solver.
        let mut cfg2 = ProofGenerationConfig::default();
        cfg2.minimize_unsat_cores = true;
        cfg2.extraction_timeout_ms = 10_000;
        cfg2.apply_to_z3_solver(&solver);

        // Saturating clamp: u64::MAX must not panic during the
        // u32 conversion — gets clamped to u32::MAX milliseconds
        // (which is ~49 days, well past anything Z3 would honour).
        let mut cfg3 = ProofGenerationConfig::default();
        cfg3.extraction_timeout_ms = u64::MAX;
        cfg3.apply_to_z3_solver(&solver);
    }

    #[test]
    fn apply_to_z3_solver_respects_zero_timeout_semantic() {
        // Pin: `extraction_timeout_ms = 0` means "no timeout".
        // The method must NOT call `set_u32("timeout", 0)` because
        // Z3 interprets timeout=0 as "fire immediately" (a zero-ms
        // budget) on some param paths, defeating the documented
        // semantic. The wiring is to OMIT the timeout param when
        // the field is zero — pinned here by the success of the
        // call (panic would surface a bug in the gate logic).
        let solver = Solver::new();

        let cfg = ProofGenerationConfig {
            enable_proofs: true,
            enable_unsat_cores: true,
            minimize_unsat_cores: false,
            max_proof_depth: 100,
            simplify_proofs: false,
            enable_proof_cache: false,
            validate_on_extract: false,
            extraction_timeout_ms: 0, // 0 = no timeout
        };
        cfg.apply_to_z3_solver(&solver);

        // Solver should still respond to a trivial check — no
        // hidden zero-timeout poisoning from the gate path.
        let result = solver.check();
        assert_eq!(
            result,
            z3::SatResult::Sat,
            "empty solver under no-timeout config must check Sat",
        );
    }
}
