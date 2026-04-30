//! Production-ready refinement type verification for Tier 1
//!
//! Verum refinement types constrain base types with predicates: five binding forms are
//! supported (inline `T{pred}`, lambda `T where |x| pred`, sigma-type `x: T where pred(x)`,
//! named predicate `T where pred_name`, bare `T where pred`). Refinement subtyping:
//! `T{P} <: T{Q}` iff `forall x. P(x) => Q(x)`. All forms desugar to sigma types
//! in the dependent type core: `(x: T, Proof(P(x)))`.
//!
//! Implements the three-tier verification strategy:
//! 1. Syntactic checks: <1ms for simple cases
//! 2. SMT solver: 10-500ms for complex predicates (with timeout)
//! 3. Runtime fallback: For undecidable cases
//!
//! Supports three verification modes:
//! - @verify(runtime): Skip SMT, use runtime checks only
//! - @verify(proof): Full SMT verification (may be slow)
//! - @verify(auto): Heuristic-based decision

use std::time::Duration;
use verum_ast::{Expr, Type, TypeKind};
use verum_common::{List, Maybe, Set, Text};
use verum_common::ToText;
use z3::ast::{Bool, Dynamic, Int, Real};

use crate::context::Context;
use crate::cost::{CostMeasurement, VerificationCost};
use crate::counterexample::{CounterExampleExtractor, generate_suggestions};
use crate::pattern_quantifiers::{PatternConfig, PatternGenerator, needs_patterns};
use crate::proof_extraction::{ProofExtractor, ProofGenerationConfig, ProofTerm};
use crate::subsumption::{CheckMode, SubsumptionChecker, SubsumptionConfig, SubsumptionResult};
use crate::translate::Translator;
use crate::verify::{
    ProofResult, VerificationError, VerificationResult, VerifyMode, estimate_complexity,
};

// ==================== Refinement Verification ====================

/// Comprehensive refinement type verifier with three-tier strategy
pub struct RefinementVerifier {
    /// SMT context for verification
    context: Context,

    /// Subsumption checker for type relationships
    subsumption: SubsumptionChecker,

    /// Verification mode (runtime, proof, or auto)
    default_mode: VerifyMode,

    /// Pattern generator for quantifier instantiation
    pattern_generator: PatternGenerator,

    /// Proof extractor for structured proof term extraction
    proof_extractor: ProofExtractor,
}

impl RefinementVerifier {
    /// Create a new refinement verifier with default configuration
    pub fn new() -> Self {
        Self::with_mode(VerifyMode::Auto)
    }

    /// Create with explicit verification mode
    pub fn with_mode(mode: VerifyMode) -> Self {
        let config = crate::context::ContextConfig {
            timeout: Some(Duration::from_secs(30)), // Spec: 30s default timeout
            ..Default::default()
        };

        let subsumption_config = SubsumptionConfig {
            cache_size: 10000,
            smt_timeout_ms: 100, // Spec: 10-500ms for subsumption, we use 100ms
        };

        let pattern_config = PatternConfig {
            enable_patterns: true, // Enable patterns by default
            ..Default::default()
        };

        // Use production config for proof extraction in proof mode,
        // development config otherwise for better performance
        let proof_config = match mode {
            VerifyMode::Proof => ProofGenerationConfig::production(),
            _ => ProofGenerationConfig::development(),
        };

        Self {
            context: Context::with_config(config),
            subsumption: SubsumptionChecker::with_config(subsumption_config),
            default_mode: mode,
            pattern_generator: PatternGenerator::new(pattern_config),
            proof_extractor: ProofExtractor::with_config(proof_config),
        }
    }

    /// Verify a refinement type constraint
    ///
    /// # Three-Tier Strategy
    ///
    /// 1. **Syntactic**: Fast pattern matching (<1ms)
    ///    - Simple comparisons: `x > 0`, `x >= y`
    ///    - Conjunction/disjunction: `a && b`, `a || b`
    ///    - Tautologies: `true`, `false`
    ///
    /// 2. **SMT**: Z3 solver with timeout (10-500ms)
    ///    - Complex predicates requiring logical reasoning
    ///    - Arithmetic relationships
    ///    - Quantified formulas (limited support)
    ///
    /// 3. **Runtime Fallback**: When SMT times out or is disabled
    ///    - Undecidable predicates
    ///    - Complex custom functions
    ///    - User-specified @verify(runtime)
    pub fn verify_refinement(
        &self,
        ty: &Type,
        value_expr: Option<&Expr>,
        mode: Option<VerifyMode>,
    ) -> VerificationResult {
        let mode = mode.unwrap_or(self.default_mode);

        // Extract refinement predicate
        let (base_type, predicate) = match &ty.kind {
            TypeKind::Refined { base, predicate } => (&**base, &predicate.expr),
            _ => {
                return Err(VerificationError::SolverError(
                    "not a refinement type".to_text(),
                ));
            }
        };

        // Check mode and decide strategy
        match mode {
            VerifyMode::Runtime => {
                // Skip SMT, return immediately for runtime checking
                let cost = VerificationCost::new("runtime_mode".into(), Duration::ZERO, true);
                Ok(ProofResult::new(cost))
            }

            VerifyMode::Proof => {
                // Full SMT verification
                self.verify_with_smt(base_type, predicate, value_expr)
            }

            VerifyMode::Auto => {
                // Heuristic decision based on complexity
                let complexity = estimate_complexity(ty);

                if complexity <= 30 {
                    // Simple - try SMT
                    self.verify_with_smt(base_type, predicate, value_expr)
                } else if complexity >= 70 {
                    // Very complex - use runtime
                    let cost = VerificationCost::new("auto_runtime".into(), Duration::ZERO, true);
                    Ok(ProofResult::new(cost))
                } else {
                    // Medium complexity - try SMT with short timeout
                    self.verify_with_smt(base_type, predicate, value_expr)
                }
            }
        }
    }

    /// Verify using SMT solver
    fn verify_with_smt(
        &self,
        base_type: &Type,
        predicate: &Expr,
        _value_expr: Option<&Expr>,
    ) -> VerificationResult {
        let measurement = CostMeasurement::start("smt_verification");

        // Create translator
        let translator = Translator::new(&self.context);

        // Create variable for the value being checked (use 'it' as convention)
        let var_name = "it";

        // Create Z3 variable for the base type
        let z3_var = translator.create_var(var_name, base_type)?;

        // Bind the variable
        let mut translator = translator;
        translator.bind(var_name.to_text(), z3_var.clone());

        // Translate the predicate
        let z3_predicate = translator.translate_expr(predicate)?;

        // Convert to boolean
        let z3_bool = z3_predicate
            .as_bool()
            .ok_or_else(|| VerificationError::SolverError("predicate is not boolean".to_text()))?;

        // Create solver and check if there exists a value that violates the constraint
        let solver = self.context.solver();

        // We want to find if there's a value where the predicate is FALSE
        // (i.e., a counterexample)
        solver.assert(z3_bool.not());

        // Check satisfiability. Route through Context::check so routing
        // stats are recorded automatically when a collector is installed
        // (see verum_build --smt-stats).
        let check_result = self.context.check(&solver);

        match check_result {
            z3::SatResult::Unsat => {
                // No counterexample exists - the constraint always holds!
                let cost = measurement.finish(true);

                // Try to extract proof if available
                let mut result = ProofResult::new(cost);

                // Extract and parse proof from solver using ProofExtractor
                if let Some(proof) = solver.get_proof() {
                    // Convert proof to Dynamic for the extractor API
                    let dynamic_proof = Dynamic::from_ast(&proof);

                    // Use ProofExtractor for proper structured proof extraction
                    match self.proof_extractor.extract_proof(&dynamic_proof) {
                        Some(proof_term) => {
                            // Calculate proof metrics
                            let proof_steps = Self::count_proof_steps(&proof_term);
                            let used_axioms = Self::extract_axiom_names(&proof_term);

                            // Serialize proof term for storage
                            let proof_str: Text = format!("{:?}", proof_term).into();
                            result = result.with_raw_proof(proof_str.clone());

                            // Create proof witness with extracted structure
                            let witness = crate::z3_backend::ProofWitness {
                                proof_term: proof_str,
                                used_axioms,
                                proof_steps,
                            };
                            result = result.with_proof_witness(witness);

                            // Store the structured proof term for downstream analysis
                            result = result.with_structured_proof(proof_term);
                        }
                        None => {
                            // Fallback to raw string representation if extraction fails
                            let proof_str: Text = format!("{:?}", proof).into();
                            result = result.with_raw_proof(proof_str.clone());

                            let witness = crate::z3_backend::ProofWitness {
                                proof_term: proof_str,
                                used_axioms: Set::new(),
                                proof_steps: 0,
                            };
                            result = result.with_proof_witness(witness);
                        }
                    }
                }

                Ok(result)
            }

            z3::SatResult::Sat => {
                // Found a counterexample - constraint can be violated
                let model = solver.get_model().ok_or_else(|| {
                    VerificationError::SolverError("no model available".to_text())
                })?;

                // Extract counterexample
                let extractor = CounterExampleExtractor::new(&model);
                let counterexample =
                    extractor.extract(&[var_name.to_text()], &format!("{:?}", predicate));

                // Generate suggestions
                let suggestions =
                    generate_suggestions(&counterexample, &format!("{:?}", predicate));

                let cost = measurement.finish(false);

                Err(VerificationError::CannotProve {
                    constraint: format!("{:?}", predicate).into(),
                    counterexample: Some(counterexample),
                    cost,
                    suggestions,
                })
            }

            z3::SatResult::Unknown => {
                // Solver couldn't determine result (timeout or too complex)
                let cost = measurement.finish(false);

                // Check if this was a timeout
                if let Some(timeout) = self.context.config().timeout
                    && cost.duration >= timeout
                {
                    return Err(VerificationError::Timeout {
                        constraint: format!("{:?}", predicate).into(),
                        timeout,
                        cost: cost.with_timeout(),
                    });
                }

                Err(VerificationError::Unknown(
                    format!("{:?}", predicate).into(),
                ))
            }
        }
    }

    /// Check subsumption: does T1{φ1} <: T2{φ2}?
    ///
    /// According to spec: T{φ₁} <: T{φ₂} iff φ₁ ⇒ φ₂
    pub fn check_subsumption(&self, ty1: &Type, ty2: &Type, mode: CheckMode) -> SubsumptionResult {
        // Extract refinement predicates
        let phi1 = match &ty1.kind {
            TypeKind::Refined { predicate, .. } => &predicate.expr,
            _ => {
                return SubsumptionResult::Unknown {
                    reason: "ty1 is not a refinement type".to_string(),
                };
            }
        };

        let phi2 = match &ty2.kind {
            TypeKind::Refined { predicate, .. } => &predicate.expr,
            _ => {
                return SubsumptionResult::Unknown {
                    reason: "ty2 is not a refinement type".to_string(),
                };
            }
        };

        // Use subsumption checker
        self.subsumption.check(phi1, phi2, mode)
    }

    /// Get subsumption cache statistics
    pub fn subsumption_stats(&self) -> crate::subsumption::SubsumptionStats {
        self.subsumption.stats()
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> crate::subsumption::CacheStats {
        self.subsumption.cache_stats()
    }

    /// Clear all caches
    pub fn clear_caches(&self) {
        self.subsumption.clear_cache();
    }

    /// Count the number of proof steps in a ProofTerm
    ///
    /// This provides a metric for proof complexity.
    fn count_proof_steps(proof: &ProofTerm) -> usize {
        match proof {
            // Leaf nodes (no sub-proofs)
            ProofTerm::Axiom { .. }
            | ProofTerm::Assumption { .. }
            | ProofTerm::Hypothesis { .. }
            | ProofTerm::Reflexivity { .. }
            | ProofTerm::TheoryLemma { .. }
            | ProofTerm::Commutativity { .. }
            | ProofTerm::DefAxiom { .. }
            | ProofTerm::DefIntro { .. }
            | ProofTerm::PullQuant { .. }
            | ProofTerm::PushQuant { .. }
            | ProofTerm::ElimUnusedVars { .. }
            | ProofTerm::Skolemize { .. }
            | ProofTerm::Distributivity { .. }
            | ProofTerm::DestructiveEqRes { .. } => 1,

            // Nodes with two sub-proofs
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => 1 + Self::count_proof_steps(premise) + Self::count_proof_steps(implication),
            ProofTerm::Transitivity { left, right } => {
                1 + Self::count_proof_steps(left) + Self::count_proof_steps(right)
            }

            // Nodes with one sub-proof
            ProofTerm::Rewrite { source, .. } => 1 + Self::count_proof_steps(source),
            ProofTerm::Symmetry { equality } => 1 + Self::count_proof_steps(equality),
            ProofTerm::QuantifierInstantiation { quantified, .. } => {
                1 + Self::count_proof_steps(quantified)
            }
            ProofTerm::Lemma { proof, .. } => 1 + Self::count_proof_steps(proof),
            ProofTerm::AndElim { conjunction, .. } => 1 + Self::count_proof_steps(conjunction),
            ProofTerm::NotOrElim {
                negated_disjunction,
                ..
            } => 1 + Self::count_proof_steps(negated_disjunction),
            ProofTerm::IffTrue { proof, .. } | ProofTerm::IffFalse { proof, .. } => {
                1 + Self::count_proof_steps(proof)
            }
            ProofTerm::ApplyDef { def_proof, .. } => 1 + Self::count_proof_steps(def_proof),
            ProofTerm::IffOEq { iff_proof, .. } => 1 + Self::count_proof_steps(iff_proof),
            ProofTerm::QuantIntro { body_proof, .. } | ProofTerm::Bind { body_proof, .. } => {
                1 + Self::count_proof_steps(body_proof)
            }

            // Nodes with list of sub-proofs
            ProofTerm::UnitResolution { clauses } | ProofTerm::HyperResolve { clauses, .. } => {
                1 + clauses.iter().map(Self::count_proof_steps).sum::<usize>()
            }
            ProofTerm::Monotonicity { premises, .. }
            | ProofTerm::NNFPos { premises, .. }
            | ProofTerm::NNFNeg { premises, .. } => {
                1 + premises.iter().map(Self::count_proof_steps).sum::<usize>()
            }
        }
    }

    /// Extract axiom names used in a proof
    ///
    /// Returns a set of all axiom names referenced in the proof.
    fn extract_axiom_names(proof: &ProofTerm) -> Set<Text> {
        let mut axioms = Set::new();
        Self::extract_axiom_names_impl(proof, &mut axioms);
        axioms
    }

    fn extract_axiom_names_impl(proof: &ProofTerm, axioms: &mut Set<Text>) {
        match proof {
            // Axiom leaf nodes
            ProofTerm::Axiom { name, .. } => {
                axioms.insert(name.clone());
            }
            ProofTerm::TheoryLemma { theory, .. } => {
                axioms.insert(theory.clone());
            }

            // Nodes with two sub-proofs
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                Self::extract_axiom_names_impl(premise, axioms);
                Self::extract_axiom_names_impl(implication, axioms);
            }
            ProofTerm::Transitivity { left, right } => {
                Self::extract_axiom_names_impl(left, axioms);
                Self::extract_axiom_names_impl(right, axioms);
            }

            // Nodes with one sub-proof
            ProofTerm::Rewrite { source, .. } => {
                Self::extract_axiom_names_impl(source, axioms);
            }
            ProofTerm::Symmetry { equality } => {
                Self::extract_axiom_names_impl(equality, axioms);
            }
            ProofTerm::QuantifierInstantiation { quantified, .. } => {
                Self::extract_axiom_names_impl(quantified, axioms);
            }
            ProofTerm::Lemma { proof, .. } => {
                Self::extract_axiom_names_impl(proof, axioms);
            }
            ProofTerm::AndElim { conjunction, .. } => {
                Self::extract_axiom_names_impl(conjunction, axioms);
            }
            ProofTerm::NotOrElim {
                negated_disjunction,
                ..
            } => {
                Self::extract_axiom_names_impl(negated_disjunction, axioms);
            }
            ProofTerm::IffTrue { proof, .. } | ProofTerm::IffFalse { proof, .. } => {
                Self::extract_axiom_names_impl(proof, axioms);
            }
            ProofTerm::ApplyDef { def_proof, .. } => {
                Self::extract_axiom_names_impl(def_proof, axioms);
            }
            ProofTerm::IffOEq { iff_proof, .. } => {
                Self::extract_axiom_names_impl(iff_proof, axioms);
            }
            ProofTerm::QuantIntro { body_proof, .. } | ProofTerm::Bind { body_proof, .. } => {
                Self::extract_axiom_names_impl(body_proof, axioms);
            }

            // Nodes with list of sub-proofs
            ProofTerm::UnitResolution { clauses } | ProofTerm::HyperResolve { clauses, .. } => {
                for clause in clauses.iter() {
                    Self::extract_axiom_names_impl(clause, axioms);
                }
            }
            ProofTerm::Monotonicity { premises, .. }
            | ProofTerm::NNFPos { premises, .. }
            | ProofTerm::NNFNeg { premises, .. } => {
                for premise in premises.iter() {
                    Self::extract_axiom_names_impl(premise, axioms);
                }
            }

            // Leaf nodes with no sub-proofs (no axiom names to extract)
            ProofTerm::Assumption { .. }
            | ProofTerm::Hypothesis { .. }
            | ProofTerm::Reflexivity { .. }
            | ProofTerm::Commutativity { .. }
            | ProofTerm::DefAxiom { .. }
            | ProofTerm::DefIntro { .. }
            | ProofTerm::PullQuant { .. }
            | ProofTerm::PushQuant { .. }
            | ProofTerm::ElimUnusedVars { .. }
            | ProofTerm::Skolemize { .. }
            | ProofTerm::Distributivity { .. }
            | ProofTerm::DestructiveEqRes { .. } => {}
        }
    }
}

impl Default for RefinementVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl RefinementVerifier {
    /// Propagate constraints through type hierarchy
    ///
    /// Given a type with refinements, propagate the constraints to
    /// extract implied predicates. This enables better verification
    /// by discovering implicit constraints.
    pub fn propagate_constraints(&self, ty: &Type) -> List<Expr> {
        let mut constraints = List::new();

        match &ty.kind {
            TypeKind::Refined { base, predicate } => {
                // Add the direct predicate
                constraints.push(predicate.expr.clone());

                // Recursively propagate from base type
                let base_constraints = self.propagate_constraints(base);
                constraints.extend(base_constraints);
            }
            TypeKind::Generic { base, args } => {
                // Propagate from base type
                let base_constraints = self.propagate_constraints(base);
                constraints.extend(base_constraints);

                // Propagate from type arguments
                for arg in args {
                    if let verum_ast::ty::GenericArg::Type(t) = arg {
                        let arg_constraints = self.propagate_constraints(t);
                        constraints.extend(arg_constraints);
                    }
                }
            }
            _ => {
                // No constraints for base types
            }
        }

        constraints
    }

    /// Verify dependent refinement types
    ///
    /// Dependent types have refinements that depend on other values.
    /// Example: List<T, n: Int> where the length is part of the type.
    ///
    /// This version uses pattern-based quantifier instantiation when beneficial.
    pub fn verify_dependent_refinement(
        &mut self,
        ty: &Type,
        dependencies: &[(Text, Expr)],
    ) -> VerificationResult {
        let measurement = CostMeasurement::start("dependent_refinement");

        // Extract refinement predicate
        let predicate = match extract_predicate(ty) {
            Some(p) => p,
            None => {
                let cost = measurement.finish(true);
                return Ok(ProofResult::new(cost));
            }
        };

        // Check if patterns would help
        let use_patterns = needs_patterns(ty);

        // Create translator
        let translator = Translator::new(&self.context);
        let mut translator = translator;

        // Bind all dependency variables. Pre-fix every dependency
        // was created as `Int::new_const(name)` regardless of the
        // value expression's actual sort — Bool / Real dependencies
        // (e.g. `Vector<f64, n: Real>`, `Validated<is_valid: Bool>`)
        // dropped silently because the `as_int()` guard skipped the
        // equality assertion. The predicate then translated against
        // an unconstrained Int constant that wasn't tied to the
        // value, producing trivially-true (unsound) verdicts.
        //
        // Dispatch on the translated value's sort and create a fresh
        // const of the matching sort. Sorts the translator can
        // currently produce: Bool, Int, Real. Other sorts fall back
        // to the legacy Int default with a tracing warning so the
        // gap is visible without a regression.
        let mut bound_vars = List::new();
        for (name, value_expr) in dependencies {
            let z3_value = match translator.translate_expr(value_expr) {
                Ok(v) => v,
                Err(_) => {
                    // Translation failed — fall back to an Int
                    // constant so the predicate at least sees a
                    // stable binding shape (the assertion is
                    // omitted; predicate translation may then fail
                    // separately, which is the intended visible
                    // failure mode).
                    let var = Int::new_const(name.as_str());
                    translator.bind(name.clone(), Dynamic::from_ast(&var));
                    bound_vars.push((name.clone(), Dynamic::from_ast(&var)));
                    continue;
                }
            };

            let solver = self.context.solver();

            if let Some(z3_bool) = z3_value.as_bool() {
                let var = Bool::new_const(name.as_str());
                solver.assert(var.eq(&z3_bool));
                let var_dyn = Dynamic::from_ast(&var);
                translator.bind(name.clone(), var_dyn.clone());
                bound_vars.push((name.clone(), var_dyn));
            } else if let Some(z3_real) = z3_value.as_real() {
                let var = Real::new_const(name.as_str());
                solver.assert(var.eq(&z3_real));
                let var_dyn = Dynamic::from_ast(&var);
                translator.bind(name.clone(), var_dyn.clone());
                bound_vars.push((name.clone(), var_dyn));
            } else if let Some(z3_int) = z3_value.as_int() {
                let var = Int::new_const(name.as_str());
                solver.assert(var.eq(&z3_int));
                let var_dyn = Dynamic::from_ast(&var);
                translator.bind(name.clone(), var_dyn.clone());
                bound_vars.push((name.clone(), var_dyn));
            } else {
                tracing::warn!(
                    "verify_dependent_refinement: dependency {:?} has unsupported \
                     Z3 sort (not Bool/Int/Real) — falling back to Int default; \
                     equality assertion omitted, predicate may translate \
                     against an unconstrained constant",
                    name.as_str()
                );
                let var = Int::new_const(name.as_str());
                let var_dyn = Dynamic::from_ast(&var);
                translator.bind(name.clone(), var_dyn.clone());
                bound_vars.push((name.clone(), var_dyn));
            }
        }

        // Translate predicate
        let z3_predicate = translator.translate_expr(predicate)?;
        let z3_bool = z3_predicate
            .as_bool()
            .ok_or_else(|| VerificationError::SolverError("predicate is not boolean".to_text()))?;

        // Apply patterns if beneficial
        let final_predicate = if use_patterns && !bound_vars.is_empty() {
            // Generate patterns for the predicate
            let bound_var_types: List<(&str, &Type)> = dependencies
                .iter()
                .map(|(name, _)| (name.as_str(), ty))
                .collect();

            let patterns =
                self.pattern_generator
                    .generate_patterns(&bound_var_types, predicate, Maybe::None);

            if !patterns.is_empty() {
                // Create quantified formula with patterns
                let patterns_vec: List<z3::Pattern> = patterns.into_iter().collect();
                if let Maybe::Some(quantified) = self.pattern_generator.mk_quantified_property(
                    &translator,
                    &bound_vars,
                    &z3_bool,
                    &patterns_vec,
                    true, // universal quantifier
                ) {
                    self.pattern_generator.stats().record_success();
                    quantified
                } else {
                    z3_bool
                }
            } else {
                z3_bool
            }
        } else {
            z3_bool
        };

        // Check if the predicate holds given the dependencies. Route
        // through Context::check so routing stats are recorded.
        let solver = self.context.solver();
        solver.assert(final_predicate.not());

        match self.context.check(&solver) {
            z3::SatResult::Unsat => {
                let cost = measurement.finish(true);
                Ok(ProofResult::new(cost))
            }
            z3::SatResult::Sat => {
                let cost = measurement.finish(false);
                if use_patterns {
                    self.pattern_generator.stats().record_failure();
                }
                Err(VerificationError::CannotProve {
                    constraint: format!("{:?}", predicate).into(),
                    counterexample: None,
                    cost,
                    suggestions: {
                        let mut list = List::new();
                        list.push("Check dependency values".to_text());
                        list.push("Strengthen refinement constraint".to_text());
                        list
                    },
                })
            }
            z3::SatResult::Unknown => {
                let _cost = measurement.finish(false);
                if use_patterns {
                    self.pattern_generator.stats().record_failure();
                }
                Err(VerificationError::Unknown(
                    format!("{:?}", predicate).into(),
                ))
            }
        }
    }

    /// Get pattern generator statistics
    pub fn pattern_stats(&self) -> &crate::pattern_quantifiers::PatternStats {
        self.pattern_generator.stats()
    }

    /// Reset pattern statistics
    pub fn reset_pattern_stats(&mut self) {
        self.pattern_generator.reset_stats();
    }

    /// Validate a proof witness
    ///
    /// Takes a proof witness and validates its structure and soundness.
    /// Returns a ProofValidation result with any errors or warnings.
    ///
    /// Note: This performs structural validation based on the proof metadata.
    /// For full proof validation including inference step verification, the proof
    /// should be validated during extraction via `ProofExtractor::validate_proof()`.
    pub fn validate_proof_witness(
        &self,
        witness: &crate::z3_backend::ProofWitness,
    ) -> crate::proof_extraction::ProofValidation {
        use crate::proof_extraction::ProofValidation;
        use verum_common::List;

        let mut validation = ProofValidation {
            is_valid: true,
            errors: List::new(),
            warnings: List::new(),
        };

        // Check basic properties - proof term must not be empty
        if witness.proof_term.is_empty() {
            validation.is_valid = false;
            validation.errors.push("Proof term is empty".to_text());
            return validation;
        }

        // Validate proof has recorded steps (indicates successful parsing)
        if witness.proof_steps == 0 {
            // This could indicate either:
            // 1. The proof is trivial (single axiom)
            // 2. Proof parsing failed to count steps
            validation
                .warnings
                .push("Proof has no steps recorded - may indicate parsing issue".to_text());
        }

        // Check for axiom information (quality metric)
        if witness.used_axioms.is_empty() {
            // Axiom-free proofs are unusual - may indicate incomplete extraction
            validation
                .warnings
                .push("No axioms recorded in proof - proof extraction may be incomplete".to_text());
        }

        // Validate proof complexity metrics for reasonable values
        const MAX_REASONABLE_PROOF_STEPS: usize = 100_000;
        if witness.proof_steps > MAX_REASONABLE_PROOF_STEPS {
            validation.warnings.push(
                format!(
                    "Proof has {} steps which exceeds reasonable limit ({}) - consider simplifying",
                    witness.proof_steps, MAX_REASONABLE_PROOF_STEPS
                )
                .into(),
            );
        }

        // Check axiom count for sanity
        const MAX_REASONABLE_AXIOMS: usize = 1_000;
        if witness.used_axioms.len() > MAX_REASONABLE_AXIOMS {
            validation.warnings.push(
                format!(
                    "Proof uses {} axioms which is unusually high - may indicate proof bloat",
                    witness.used_axioms.len()
                )
                .into(),
            );
        }

        // Check for known problematic patterns in proof term
        if witness.proof_term.contains("unknown:") {
            validation
                .warnings
                .push("Proof contains unknown proof rules - may not be fully verified".to_text());
        }

        validation
    }

    /// Extract and validate proof from a verification result
    ///
    /// This is a convenience method that combines proof extraction and validation.
    pub fn extract_and_validate_proof(
        &self,
        result: &mut ProofResult,
    ) -> Maybe<crate::proof_extraction::ProofValidation> {
        if let Some(witness) = &result.proof_witness {
            let validation = self.validate_proof_witness(witness);
            Some(validation)
        } else {
            None
        }
    }
}

// ==================== Helper Functions ====================

/// Quick check if a type needs SMT verification
pub fn needs_smt_verification(ty: &Type, mode: VerifyMode) -> bool {
    match mode {
        VerifyMode::Runtime => false,
        VerifyMode::Proof => is_refinement_type(ty),
        VerifyMode::Auto => {
            if !is_refinement_type(ty) {
                return false;
            }

            let complexity = estimate_complexity(ty);
            complexity > 0 && complexity < 70
        }
    }
}

/// Check if a type is a refinement type
pub fn is_refinement_type(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::Refined { .. })
}

/// Extract the refinement predicate from a type
pub fn extract_predicate(ty: &Type) -> Option<&Expr> {
    match &ty.kind {
        TypeKind::Refined { predicate, .. } => Some(&predicate.expr),
        _ => None,
    }
}

/// Categorize a predicate by complexity
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredicateComplexity {
    /// Simple comparison (<1ms syntactic check)
    Simple,
    /// Medium complexity (10-100ms SMT)
    Medium,
    /// Complex (100-500ms SMT)
    Complex,
    /// Very complex (>500ms, suggest runtime)
    VeryComplex,
}

/// Estimate predicate complexity category
pub fn categorize_complexity(ty: &Type) -> PredicateComplexity {
    let score = estimate_complexity(ty);

    if score <= 20 {
        PredicateComplexity::Simple
    } else if score <= 50 {
        PredicateComplexity::Medium
    } else if score <= 70 {
        PredicateComplexity::Complex
    } else {
        PredicateComplexity::VeryComplex
    }
}

// ==================== Tests ====================
