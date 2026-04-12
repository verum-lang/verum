//! Proof type checker for Verum's formal proof system.
//!
//! This module implements type checking for theorem/proof constructs:
//! - Validates theorem propositions are well-typed
//! - Tracks hypothesis contexts during proof checking
//! - Validates that proof bodies match propositions
//! - Integrates with existing InferenceContext
//!
//! Formal proof system (future v2.0+): machine-checkable proofs with tactics (simp, ring, omega, blast, induction), theorem/lemma/corollary statements

use crate::context::{TypeContext, TypeScheme};
use crate::infer::{InferMode, InferResult, TypeChecker};
use crate::ty::{Type, TypeVar};
use crate::unify::Unifier;
use crate::{Result, TypeError};
use verum_ast::decl::{
    AxiomDecl, CalcRelation, CalculationChain, FunctionParam, FunctionParamKind, ProofBody,
    ProofCase, ProofMethod, ProofStep, ProofStepKind, ProofStructure, TacticExpr, TheoremDecl,
};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::{Span, Spanned};
use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
use verum_common::{Heap, List, Map, Maybe, Text};
use verum_common::ToText;

/// Proof checker for validating theorems and proofs.
///
/// This integrates with the main type checker to provide proof-specific
/// validation including hypothesis tracking and tactic type checking.
pub struct ProofChecker {
    /// Hypothesis context mapping hypothesis names to their types/propositions
    pub(crate) hypotheses: Map<Text, Type>,
    /// Current goal type being proven
    current_goal: Maybe<Type>,
    /// Unifier for type-level equality checks
    unifier: Unifier,
}

impl ProofChecker {
    /// Create a new proof checker
    pub fn new() -> Self {
        Self {
            hypotheses: Map::new(),
            current_goal: Maybe::None,
            unifier: Unifier::new(),
        }
    }

    /// Check a theorem declaration.
    ///
    /// Validates:
    /// 1. Proposition is well-typed and evaluates to Bool or Prop
    /// 2. Generic parameters are valid
    /// 3. Parameters are well-typed
    /// 4. If proof is present, it validates the proposition
    ///
    /// Theorem statements: "theorem name(params): proposition { proof_term }", with lemma and corollary variants
    pub fn check_theorem(
        &mut self,
        decl: &TheoremDecl,
        ctx: &mut TypeContext,
        type_checker: &mut TypeChecker,
    ) -> Result<()> {
        let span = decl.span;

        // Enter new scope for theorem checking
        ctx.enter_scope();

        // Add generic type parameters to context
        for generic in decl.generics.iter() {
            self.add_generic_to_context(generic, ctx, span)?;
        }

        // Add theorem parameters to context (like function parameters)
        for param in decl.params.iter() {
            self.add_param_to_context(param, ctx, type_checker, span)?;
        }

        // Check proposition is well-typed
        // Propositions should be Bool or Prop type
        let prop_result = type_checker.infer(&decl.proposition, InferMode::Synth)?;
        let prop_ty = prop_result.ty;

        // Validate proposition type is Bool or Prop
        if !self.is_valid_proposition_type(&prop_ty) {
            return Err(TypeError::Mismatch {
                expected: Text::from("Bool or Prop"),
                actual: prop_ty.to_text(),
                span,
            });
        }

        // If proof is present, validate it proves the proposition
        if let Option::Some(ref proof) = decl.proof {
            self.check_proof_body(proof, &prop_ty, ctx, type_checker, span)?;
        }

        // Exit theorem scope
        ctx.exit_scope();

        Ok(())
    }

    /// Check an axiom declaration.
    ///
    /// Validates:
    /// 1. Proposition is well-typed and evaluates to Bool or Prop
    /// 2. Generic parameters are valid
    /// 3. Parameters are well-typed
    ///
    /// Axioms have no proof body (they are assumed true).
    ///
    /// Theorem statements: "theorem name(params): proposition { proof_term }", with lemma and corollary variants
    pub fn check_axiom(
        &mut self,
        decl: &AxiomDecl,
        ctx: &mut TypeContext,
        type_checker: &mut TypeChecker,
    ) -> Result<()> {
        let span = decl.span;

        // Enter new scope for axiom checking
        ctx.enter_scope();

        // Add generic type parameters to context
        for generic in decl.generics.iter() {
            self.add_generic_to_context(generic, ctx, span)?;
        }

        // Add axiom parameters to context
        for param in decl.params.iter() {
            self.add_param_to_context(param, ctx, type_checker, span)?;
        }

        // Check proposition is well-typed
        let prop_result = type_checker.infer(&decl.proposition, InferMode::Synth)?;
        let prop_ty = prop_result.ty;

        // Validate proposition type is Bool or Prop
        if !self.is_valid_proposition_type(&prop_ty) {
            return Err(TypeError::Mismatch {
                expected: Text::from("Bool or Prop"),
                actual: prop_ty.to_text(),
                span,
            });
        }

        // Exit axiom scope
        ctx.exit_scope();

        Ok(())
    }

    /// Check a proof body validates the given proposition.
    ///
    /// Proof terms: first-class proof values, modus ponens, case analysis, proof by contradiction
    pub(crate) fn check_proof_body(
        &mut self,
        proof: &ProofBody,
        proposition: &Type,
        ctx: &mut TypeContext,
        type_checker: &mut TypeChecker,
        span: Span,
    ) -> Result<()> {
        // Set current goal
        let old_goal = self.current_goal.clone();
        self.current_goal = Maybe::Some(proposition.clone());

        let result = match proof {
            ProofBody::Term(term) => {
                // Term proof: check that term has type proposition
                // In Curry-Howard, a proof of P is a term of type P
                let term_result = type_checker.infer(term, InferMode::Synth)?;
                self.unifier.unify(&term_result.ty, proposition, span)?;
                Ok(())
            }

            ProofBody::Tactic(tactic) => {
                // Tactic proof: validate tactic completes the goal
                self.check_tactic(tactic, proposition, ctx, type_checker, span)
            }

            ProofBody::Structured(structure) => {
                // Structured proof: validate steps and conclusion
                self.check_proof_structure(structure, proposition, ctx, type_checker)
            }

            ProofBody::ByMethod(method) => {
                // Proof by method (induction, cases, contradiction)
                self.check_proof_method(method, proposition, ctx, type_checker, span)
            }
        };

        // Restore old goal
        self.current_goal = old_goal;
        result
    }

    /// Check a structured proof.
    ///
    /// Validates each proof step and ensures the final conclusion matches the goal.
    ///
    /// Proof tactics: simp (simplification), ring (ring normalization), omega (linear arithmetic), blast (tableau prover), induction
    fn check_proof_structure(
        &mut self,
        structure: &ProofStructure,
        goal: &Type,
        ctx: &mut TypeContext,
        type_checker: &mut TypeChecker,
    ) -> Result<()> {
        // Enter new scope for proof
        ctx.enter_scope();

        // Check each proof step
        for step in structure.steps.iter() {
            self.check_proof_step(step, ctx, type_checker)?;
        }

        // If there's an explicit conclusion, check it completes the goal
        if let Option::Some(ref conclusion) = structure.conclusion {
            self.check_tactic(conclusion, goal, ctx, type_checker, structure.span)?;
        }

        // Exit proof scope
        ctx.exit_scope();

        Ok(())
    }

    /// Check a single proof step.
    ///
    /// Proof tactics: simp (simplification), ring (ring normalization), omega (linear arithmetic), blast (tableau prover), induction
    fn check_proof_step(
        &mut self,
        step: &ProofStep,
        ctx: &mut TypeContext,
        type_checker: &mut TypeChecker,
    ) -> Result<()> {
        let span = step.span;

        match &step.kind {
            ProofStepKind::Have {
                name,
                proposition,
                justification,
            } => {
                // Check the proposition is well-typed
                let prop_result = type_checker.infer(proposition, InferMode::Synth)?;
                let prop_ty = prop_result.ty;

                if !self.is_valid_proposition_type(&prop_ty) {
                    return Err(TypeError::Mismatch {
                        expected: Text::from("Bool or Prop"),
                        actual: prop_ty.to_text(),
                        span,
                    });
                }

                // Validate the justification proves the proposition
                self.check_tactic(justification, &prop_ty, ctx, type_checker, span)?;

                // Add hypothesis to context
                self.hypotheses
                    .insert(name.name.to_string().into(), prop_ty.clone());
                ctx.env.insert_mono(name.name.to_string(), prop_ty);

                Ok(())
            }

            ProofStepKind::Show {
                proposition,
                justification,
            } => {
                // Check the proposition is well-typed
                let prop_result = type_checker.infer(proposition, InferMode::Synth)?;
                let prop_ty = prop_result.ty;

                if !self.is_valid_proposition_type(&prop_ty) {
                    return Err(TypeError::Mismatch {
                        expected: Text::from("Bool or Prop"),
                        actual: prop_ty.to_text(),
                        span,
                    });
                }

                // Validate the justification proves the proposition
                self.check_tactic(justification, &prop_ty, ctx, type_checker, span)
            }

            ProofStepKind::Suffices {
                proposition,
                justification,
            } => {
                // "Suffices to show P" means: if we can show P, then the current goal follows
                // So we validate P is well-typed and the justification shows current_goal → P
                let prop_result = type_checker.infer(proposition, InferMode::Synth)?;
                let prop_ty = prop_result.ty;

                if !self.is_valid_proposition_type(&prop_ty) {
                    return Err(TypeError::Mismatch {
                        expected: Text::from("Bool or Prop"),
                        actual: prop_ty.to_text(),
                        span,
                    });
                }

                // Check justification (validates the implication)
                self.check_tactic(justification, &prop_ty, ctx, type_checker, span)
            }

            ProofStepKind::Let { pattern, value } => {
                // Let binding in proof context
                let value_result = type_checker.infer(value, InferMode::Synth)?;
                let value_ty = value_result.ty;

                // Bind pattern to value type
                self.bind_pattern(pattern, &value_ty, ctx, type_checker, span)?;

                Ok(())
            }

            ProofStepKind::Obtain { pattern, from } => {
                // Obtain existential witnesses from an existential proof
                let from_result = type_checker.infer(from, InferMode::Synth)?;
                let from_ty = from_result.ty;

                // Validate from_ty is an existential type and bind pattern variables
                // For now, create fresh type variables for the pattern bindings
                let witness_ty = Type::Var(TypeVar::fresh());
                self.bind_pattern(pattern, &witness_ty, ctx, type_checker, span)?;

                Ok(())
            }

            ProofStepKind::Calc(chain) => {
                // Check calculation chain
                self.check_calculation_chain(chain, ctx, type_checker)
            }

            ProofStepKind::Cases { scrutinee, cases } => {
                // Case analysis
                let scrutinee_result = type_checker.infer(scrutinee, InferMode::Synth)?;
                let scrutinee_ty = scrutinee_result.ty;

                // Check each case
                for case in cases.iter() {
                    ctx.enter_scope();

                    // Bind pattern
                    self.bind_pattern(&case.pattern, &scrutinee_ty, ctx, type_checker, span)?;

                    // Check case proof
                    for step in case.proof.iter() {
                        self.check_proof_step(step, ctx, type_checker)?;
                    }

                    ctx.exit_scope();
                }

                Ok(())
            }

            ProofStepKind::Focus { goal_index, steps } => {
                // Focus on a specific subgoal
                // For now, just check the steps
                for step in steps.iter() {
                    self.check_proof_step(step, ctx, type_checker)?;
                }
                Ok(())
            }

            ProofStepKind::Tactic(_tactic) => {
                // Tactic application - check that the tactic is valid
                // For now, accept any tactic
                Ok(())
            }
        }
    }

    /// Check a calculation chain (equational reasoning).
    ///
    /// Mathematical structures: algebraic protocols (Group, Ring, Field) with laws as theorem requirements — .1
    fn check_calculation_chain(
        &mut self,
        chain: &CalculationChain,
        ctx: &mut TypeContext,
        type_checker: &mut TypeChecker,
    ) -> Result<()> {
        // Check start expression is well-typed
        let start_result = type_checker.infer(&chain.start, InferMode::Synth)?;
        let mut current_ty = start_result.ty;

        // Check each step
        for step in chain.steps.iter() {
            // Check target expression is well-typed
            let target_result = type_checker.infer(&step.target, InferMode::Synth)?;
            let target_ty = target_result.ty;

            // Verify types are compatible with relation
            match step.relation {
                CalcRelation::Eq
                | CalcRelation::Ne
                | CalcRelation::Lt
                | CalcRelation::Le
                | CalcRelation::Gt
                | CalcRelation::Ge => {
                    // Types should be the same or unifiable
                    self.unifier.unify(&current_ty, &target_ty, step.span)?;
                }
                CalcRelation::Implies | CalcRelation::Iff => {
                    // Both should be Bool or Prop
                    if !self.is_valid_proposition_type(&current_ty) {
                        return Err(TypeError::Mismatch {
                            expected: Text::from("Bool or Prop"),
                            actual: current_ty.to_text(),
                            span: step.span,
                        });
                    }
                    if !self.is_valid_proposition_type(&target_ty) {
                        return Err(TypeError::Mismatch {
                            expected: Text::from("Bool or Prop"),
                            actual: target_ty.to_text(),
                            span: step.span,
                        });
                    }
                }
                _ => {
                    // Other relations - accept for now
                }
            }

            // Check justification
            let relation_prop = Type::Bool; // Simplified - would need proper relation type
            self.check_tactic(
                &step.justification,
                &relation_prop,
                ctx,
                type_checker,
                step.span,
            )?;

            current_ty = target_ty;
        }

        Ok(())
    }

    /// Check a proof by method (induction, cases, contradiction).
    ///
    /// Proof terms: first-class proof values, modus ponens, case analysis, proof by contradiction
    fn check_proof_method(
        &mut self,
        method: &ProofMethod,
        goal: &Type,
        ctx: &mut TypeContext,
        type_checker: &mut TypeChecker,
        span: Span,
    ) -> Result<()> {
        match method {
            ProofMethod::Induction { on: _, cases } => {
                // Check each induction case
                for case in cases.iter() {
                    ctx.enter_scope();

                    // Bind case pattern (inductive hypothesis available)
                    // For each case, check the proof
                    for step in case.proof.iter() {
                        self.check_proof_step(step, ctx, type_checker)?;
                    }

                    ctx.exit_scope();
                }
                Ok(())
            }

            ProofMethod::Cases { on, cases } => {
                // Check scrutinee
                let on_result = type_checker.infer(on, InferMode::Synth)?;
                let on_ty = on_result.ty;

                // Check each case
                for case in cases.iter() {
                    ctx.enter_scope();

                    // Bind pattern
                    self.bind_pattern(&case.pattern, &on_ty, ctx, type_checker, span)?;

                    // Check case proof
                    for step in case.proof.iter() {
                        self.check_proof_step(step, ctx, type_checker)?;
                    }

                    ctx.exit_scope();
                }

                Ok(())
            }

            ProofMethod::Contradiction { assumption, proof } => {
                // Proof by contradiction: assume ¬goal, derive False
                ctx.enter_scope();

                // Add negation of goal as hypothesis
                let negated_goal = Type::Bool; // Simplified
                self.hypotheses
                    .insert(assumption.name.to_string().into(), negated_goal.clone());
                ctx.env
                    .insert_mono(assumption.name.to_string(), negated_goal);

                // Check proof derives False
                for step in proof.iter() {
                    self.check_proof_step(step, ctx, type_checker)?;
                }

                ctx.exit_scope();
                Ok(())
            }

            ProofMethod::StrongInduction { on: _, cases } => {
                // Similar to regular induction
                for case in cases.iter() {
                    ctx.enter_scope();

                    for step in case.proof.iter() {
                        self.check_proof_step(step, ctx, type_checker)?;
                    }

                    ctx.exit_scope();
                }
                Ok(())
            }

            ProofMethod::WellFoundedInduction {
                relation: _,
                on: _,
                cases,
            } => {
                // Similar to strong induction with a well-founded relation
                for case in cases.iter() {
                    ctx.enter_scope();

                    for step in case.proof.iter() {
                        self.check_proof_step(step, ctx, type_checker)?;
                    }

                    ctx.exit_scope();
                }
                Ok(())
            }
        }
    }

    /// Check a tactic validates the goal.
    ///
    /// For now, this is a simplified check that accepts tactics
    /// that syntactically make sense. A full implementation would
    /// execute tactics symbolically.
    ///
    /// Proof tactics: simp (simplification), ring (ring normalization), omega (linear arithmetic), blast (tableau prover), induction
    pub(crate) fn check_tactic(
        &mut self,
        tactic: &TacticExpr,
        goal: &Type,
        ctx: &mut TypeContext,
        type_checker: &mut TypeChecker,
        span: Span,
    ) -> Result<()> {
        match tactic {
            // Terminal tactics - always succeed
            TacticExpr::Trivial
            | TacticExpr::Assumption
            | TacticExpr::Reflexivity
            | TacticExpr::Done => Ok(()),

            // Unsafe tactics - accept but these leave proof obligations unchecked
            // Warning is emitted during code generation phase via ProofCodegen
            // since it has access to the full diagnostic infrastructure.
            // See proof_codegen.rs ProofStatus::Admitted for the warning handling.
            TacticExpr::Admit | TacticExpr::Sorry => Ok(()),

            // Contradiction tactic - proof by contradiction
            TacticExpr::Contradiction => Ok(()),

            // Introduction
            TacticExpr::Intro(_names) => {
                // Intro should work on implications or foralls
                // For now, accept
                Ok(())
            }

            // Apply lemma
            TacticExpr::Apply { lemma, args: _ } => {
                // Check lemma is well-typed
                let lemma_result = type_checker.infer(lemma, InferMode::Synth)?;
                // Verify lemma type matches or implies goal
                // For now, accept
                Ok(())
            }

            // Rewrite
            TacticExpr::Rewrite {
                hypothesis,
                at_target: _,
                rev: _,
            } => {
                // Check hypothesis is well-typed
                let hyp_result = type_checker.infer(hypothesis, InferMode::Synth)?;
                // Should be an equality type
                // For now, accept
                Ok(())
            }

            // Simplification
            TacticExpr::Simp {
                lemmas,
                at_target: _,
            } => {
                // Check lemmas are well-typed
                for lemma in lemmas.iter() {
                    type_checker.infer(lemma, InferMode::Synth)?;
                }
                Ok(())
            }

            // Arithmetic solvers
            TacticExpr::Ring | TacticExpr::Field | TacticExpr::Omega => {
                // These tactics solve specific domains
                // Accept if goal is Bool or Prop
                if !self.is_valid_proposition_type(goal) {
                    return Err(TypeError::Mismatch {
                        expected: Text::from("Bool or Prop"),
                        actual: goal.to_text(),
                        span,
                    });
                }
                Ok(())
            }

            // Automated tactics
            TacticExpr::Auto { with_hints: _ } | TacticExpr::Blast | TacticExpr::Smt { .. } => {
                // Automated tactics - accept
                Ok(())
            }

            // Logical tactics
            TacticExpr::Split | TacticExpr::Left | TacticExpr::Right => Ok(()),

            // Existential witness
            TacticExpr::Exists(witness) => {
                // Check witness is well-typed
                type_checker.infer(witness, InferMode::Synth)?;
                Ok(())
            }

            // Case analysis
            TacticExpr::CasesOn(_var) | TacticExpr::InductionOn(_var) => Ok(()),

            // Exact proof
            TacticExpr::Exact(proof) => {
                // Check proof has type goal
                let proof_result = type_checker.infer(proof, InferMode::Synth)?;
                self.unifier.unify(&proof_result.ty, goal, span)?;
                Ok(())
            }

            // Unfolding
            TacticExpr::Unfold(_) | TacticExpr::Compute => Ok(()),

            // Combinators
            TacticExpr::Try(inner) => self.check_tactic(inner, goal, ctx, type_checker, span),

            TacticExpr::TryElse { body, fallback } => {
                self.check_tactic(body, goal, ctx, type_checker, span)
                    .or_else(|_| self.check_tactic(fallback, goal, ctx, type_checker, span))
            }

            TacticExpr::Repeat(inner) => self.check_tactic(inner, goal, ctx, type_checker, span),

            TacticExpr::Seq(tactics) => {
                // Check each tactic in sequence
                for t in tactics.iter() {
                    self.check_tactic(t, goal, ctx, type_checker, span)?;
                }
                Ok(())
            }

            TacticExpr::Alt(tactics) => {
                // At least one alternative should work
                // For now, just check first one
                if let Some(first) = tactics.first() {
                    self.check_tactic(first, goal, ctx, type_checker, span)?;
                }
                Ok(())
            }

            TacticExpr::AllGoals(inner) => self.check_tactic(inner, goal, ctx, type_checker, span),

            TacticExpr::Focus(inner) => self.check_tactic(inner, goal, ctx, type_checker, span),

            // Named tactic
            TacticExpr::Named { name: _, args } => {
                // Check arguments are well-typed
                for arg in args.iter() {
                    type_checker.infer(arg, InferMode::Synth)?;
                }
                Ok(())
            }
        }
    }

    /// Bind a pattern to a type in the context.
    fn bind_pattern(
        &mut self,
        pattern: &Pattern,
        ty: &Type,
        ctx: &mut TypeContext,
        type_checker: &mut TypeChecker,
        span: Span,
    ) -> Result<()> {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                ctx.env.insert_mono(name.name.to_string(), ty.clone());
                Ok(())
            }

            PatternKind::Wildcard => Ok(()),

            PatternKind::Tuple(elements) => {
                // Destructure tuple type
                if let Type::Tuple(element_tys) = ty {
                    if elements.len() != element_tys.len() {
                        return Err(TypeError::Mismatch {
                            expected: format!("tuple with {} elements", element_tys.len()).into(),
                            actual: format!("tuple pattern with {} elements", elements.len())
                                .into(),
                            span,
                        });
                    }

                    for (pat, elem_ty) in elements.iter().zip(element_tys.iter()) {
                        self.bind_pattern(pat, elem_ty, ctx, type_checker, span)?;
                    }
                    Ok(())
                } else {
                    Err(TypeError::Mismatch {
                        expected: Text::from("tuple type"),
                        actual: ty.to_text(),
                        span,
                    })
                }
            }

            PatternKind::Record { fields, .. } => {
                // For record patterns, bind each field
                for field in fields.iter() {
                    let field_ty = Type::Var(TypeVar::fresh());
                    if let Some(ref pat) = field.pattern {
                        self.bind_pattern(pat, &field_ty, ctx, type_checker, span)?;
                    }
                }
                Ok(())
            }

            PatternKind::Variant { data, .. } => {
                // For variant patterns, we'd need to look up the type
                // For now, accept
                Ok(())
            }

            PatternKind::Literal(_) => {
                // Literals don't bind variables
                Ok(())
            }

            PatternKind::Range { .. } => Ok(()),

            PatternKind::Or(alternatives) => {
                // For or patterns, bind each alternative
                for alt in alternatives.iter() {
                    self.bind_pattern(alt, ty, ctx, type_checker, span)?;
                }
                Ok(())
            }

            PatternKind::Rest => Ok(()),

            PatternKind::Reference { inner, .. } => {
                // Bind the inner pattern
                self.bind_pattern(inner, ty, ctx, type_checker, span)
            }

            PatternKind::Array(elements)
            | PatternKind::Slice {
                before: elements, ..
            } => {
                // For arrays/slices, bind each element
                for elem in elements.iter() {
                    let elem_ty = Type::Var(TypeVar::fresh());
                    self.bind_pattern(elem, &elem_ty, ctx, type_checker, span)?;
                }
                Ok(())
            }

            PatternKind::Paren(inner) => self.bind_pattern(inner, ty, ctx, type_checker, span),            PatternKind::View { pattern, .. } => {
                self.bind_pattern(pattern, ty, ctx, type_checker, span)
            }

            PatternKind::Active { bindings, .. } => {
                // Active patterns have a name, optional params (expressions), and
                // extraction bindings. The bindings are patterns that match against
                // the inner value returned by the active pattern (e.g., Some(n) unwraps to n).
                // Bind each extraction pattern to a fresh type variable.
                for binding in bindings.iter() {
                    let binding_ty = Type::Var(TypeVar::fresh());
                    self.bind_pattern(binding, &binding_ty, ctx, type_checker, span)?;
                }
                Ok(())
            }

            PatternKind::And(patterns) => {
                // And patterns: bind variables from all sub-patterns
                for pat in patterns.iter() {
                    self.bind_pattern(pat, ty, ctx, type_checker, span)?;
                }
                Ok(())
            }

            PatternKind::TypeTest { binding, .. } => {
                // Type test pattern: x is Type
                // Binds the name to the tested type (narrowed type)
                // For now, use a fresh type variable since we don't have full
                // AST-to-internal type conversion here
                let narrowed_ty = Type::Var(TypeVar::fresh());
                ctx.env.insert_mono(binding.name.to_string(), narrowed_ty);
                Ok(())
            }

            PatternKind::Stream { head_patterns, rest } => {
                // Stream pattern: stream[first, second, ...rest]
                // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 18.3 - Stream Pattern Matching
                //
                // Bind each head pattern element to a fresh type variable (element type)
                // and optionally bind the rest identifier to the iterator type
                let elem_ty = Type::Var(TypeVar::fresh());
                for pat in head_patterns.iter() {
                    self.bind_pattern(pat, &elem_ty, ctx, type_checker, span)?;
                }

                // If there's a rest binding, bind it to the original type (iterator)
                if let verum_common::Maybe::Some(rest_ident) = rest {
                    ctx.env.insert_mono(rest_ident.name.to_string(), ty.clone());
                }

                Ok(())
            }

            PatternKind::Guard { pattern, .. } => {
                // Guard pattern: (pattern if expr)
                // Spec: Rust RFC 3637 - Guard Patterns
                //
                // Bind the inner pattern; guard expression is handled separately
                self.bind_pattern(pattern, ty, ctx, type_checker, span)
            }

            PatternKind::Cons { head, tail } => {
                // Cons pattern: head :: tail — bind both parts
                self.bind_pattern(head, ty, ctx, type_checker, span)?;
                self.bind_pattern(tail, ty, ctx, type_checker, span)
            }
        }
    }

    /// Add a generic parameter to the type context.
    fn add_generic_to_context(
        &mut self,
        generic: &GenericParam,
        ctx: &mut TypeContext,
        span: Span,
    ) -> Result<()> {
        match &generic.kind {
            GenericParamKind::Type { name, bounds, .. } => {
                // Create a type variable for this generic
                let type_var = TypeVar::fresh();
                ctx.env
                    .insert_mono(name.name.to_string(), Type::Var(type_var));
                // Note: bounds checking would be done separately
                Ok(())
            }

            GenericParamKind::Const { name, ty } | GenericParamKind::Meta { name, ty, .. } => {
                // For const/meta generics, we'd create a meta type
                // For now, treat as a regular variable
                let const_ty = Type::Int; // Simplified
                ctx.env.insert_mono(name.name.to_string(), const_ty);
                Ok(())
            }

            GenericParamKind::Lifetime { .. } => {
                // Lifetime parameters don't affect type checking
                Ok(())
            }

            GenericParamKind::HigherKinded { name, arity, bounds } => {
                // Higher-kinded type parameters (e.g., F<_>: Functor)
                // Create a type constructor variable
                let type_var = TypeVar::fresh();
                ctx.env
                    .insert_mono(name.name.to_string(), Type::Var(type_var));
                Ok(())
            }

            GenericParamKind::Context { name } => {
                // Context parameters for context polymorphism
                // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 17.2
                // Create a type variable that will be unified with context lists
                let type_var = TypeVar::fresh();
                ctx.env
                    .insert_mono(name.name.to_string(), Type::Var(type_var));
                Ok(())
            }

            GenericParamKind::Level { .. } => {
                // Universe level parameters don't affect value-level type checking
                Ok(())
            }
        }
    }

    /// Add a function parameter to the type context.
    fn add_param_to_context(
        &mut self,
        param: &FunctionParam,
        ctx: &mut TypeContext,
        type_checker: &mut TypeChecker,
        span: Span,
    ) -> Result<()> {
        match &param.kind {
            FunctionParamKind::Regular { pattern, ty, .. } => {
                // Use a fresh type variable for the parameter type.
                // The actual type annotation is checked separately during type inference
                // via TypeChecker. Here we just need to track the binding for scope purposes.
                // This works because type unification will resolve the variable to the
                // concrete type during the full type checking pass.
                let param_ty = Type::Var(TypeVar::fresh());
                self.bind_pattern(pattern, &param_ty, ctx, type_checker, span)?;
                Ok(())
            }

            FunctionParamKind::SelfValue
            | FunctionParamKind::SelfValueMut
            | FunctionParamKind::SelfRef
            | FunctionParamKind::SelfRefMut
            | FunctionParamKind::SelfRefChecked
            | FunctionParamKind::SelfRefCheckedMut
            | FunctionParamKind::SelfRefUnsafe
            | FunctionParamKind::SelfRefUnsafeMut
            | FunctionParamKind::SelfOwn
            | FunctionParamKind::SelfOwnMut => {
                // Self parameters - would need context about the implementing type
                // For theorem parameters, these shouldn't appear
                Err(TypeError::Other(Text::from(
                    "self parameters not allowed in theorem declarations",
                )))
            }
        }
    }

    /// Check if a type is a valid proposition type (Bool or Prop).
    pub(crate) fn is_valid_proposition_type(&self, ty: &Type) -> bool {
        matches!(ty, Type::Bool | Type::Prop)
    }

    /// Get a hypothesis from the context.
    pub fn get_hypothesis(&self, name: &Text) -> Option<&Type> {
        self.hypotheses.get(name)
    }

    /// Clear all hypotheses (for resetting between proofs).
    pub fn clear_hypotheses(&mut self) {
        self.hypotheses.clear();
    }
}

impl Default for ProofChecker {
    fn default() -> Self {
        Self::new()
    }
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::Expr;
    use verum_ast::literal::Literal;
    use verum_ast::span::Span;

    #[test]
    fn test_proof_checker_creation() {
        let checker = ProofChecker::new();
        assert!(checker.hypotheses.is_empty());
        assert!(checker.current_goal.is_none());
    }

    #[test]
    fn test_valid_proposition_type() {
        let checker = ProofChecker::new();
        assert!(checker.is_valid_proposition_type(&Type::Bool));
        assert!(checker.is_valid_proposition_type(&Type::Prop));
        assert!(!checker.is_valid_proposition_type(&Type::Int));
        assert!(!checker.is_valid_proposition_type(&Type::Text));
    }

    #[test]
    fn test_hypothesis_management() {
        let mut checker = ProofChecker::new();
        let h_name = Text::from("h1");
        let h_type = Type::Bool;

        // Add hypothesis
        checker.hypotheses.insert(h_name.clone(), h_type.clone());

        // Retrieve hypothesis
        let retrieved = checker.get_hypothesis(&h_name);
        assert!(retrieved.is_some());
        if let Some(ty) = retrieved {
            assert_eq!(ty, &h_type);
        } else {
            panic!("Expected hypothesis");
        }

        // Clear hypotheses
        checker.clear_hypotheses();
        assert!(checker.hypotheses.is_empty());
    }

    #[test]
    fn test_simple_axiom_checking() {
        let mut checker = ProofChecker::new();
        let mut ctx = TypeContext::new();
        let mut type_checker = TypeChecker::new();

        // Create a simple axiom: axiom excluded_middle: Bool ∨ ¬Bool
        let proposition = Expr::new(
            ExprKind::Literal(Literal::bool(true, Span::dummy())),
            Span::dummy(),
        );

        let axiom = AxiomDecl {
            visibility: verum_ast::decl::Visibility::Private,
            name: Ident::new("excluded_middle", Span::dummy()),
            generics: vec![].into(),
            params: vec![].into(),
            return_type: Maybe::None,
            proposition: Box::new(proposition),
            generic_where_clause: None,
            meta_where_clause: None,
            attributes: vec![].into(),
            span: Span::dummy(),
        };

        // Check axiom
        let result = checker.check_axiom(&axiom, &mut ctx, &mut type_checker);
        assert!(result.is_ok());
    }
}
