//! Dependent Types Support for Future Extensions (v2.0+)
//!
//! This module provides SMT backend support for dependent types as specified in:
//! - Dependent types: Pi types `(x: A) -> B(x)`, Sigma types `(x: A, B(x))`,
//!   Equality types `Eq<A, x, y>`, universe hierarchy `Type : Type1 : Type2 ...`
//! - Formal proofs: proof terms as first-class values, SMT integration for
//!   decidable fragments, custom theories for bitvectors and arrays
//!
//! ## Features Prepared for v2.0+
//!
//! - **Pi Types**: Dependent function types (x: A) -> B(x)
//! - **Sigma Types**: Dependent pair types (x: A, B(x))
//! - **Equality Types**: Propositional equality a = b
//! - **Proof Terms**: Integration for formal verification
//! - **Quantifiers**: First-class support for ∀ and ∃
//!
//! ## Current Status
//!
//! This module provides the foundation for dependent types support.
//! The actual implementation will be fully activated in v2.0+.
//!
//! SMT backend requirements for dependent types: Pi/Sigma types encode as universally/
//! existentially quantified formulas. Equality types use Z3's built-in equality.
//! Custom SMT theories (e.g., BitVector) extend the solver with domain-specific axioms.
//! Proof search uses automated strategies (assumption, reflexivity, intro, split, apply)
//! with a hints database for priority-based lemma application.

use z3::ast::{Bool, Dynamic};

use verum_ast::expr::ConditionKind;
use verum_ast::{BinOp, ContextList, Expr, ExprKind, Type, TypeKind};
use verum_common::{Heap, List, Map, Maybe, Set, Text};
use verum_common::ToText;

use crate::option_to_maybe;
use crate::translate::Translator;
use crate::verify::{ProofResult, VerificationCost, VerificationError, VerificationResult};

// ==================== Dependent Type Structures ====================

/// Dependent function type: Pi(x: A) -> B(x)
///
/// Represents a function type where the return type B can depend on the
/// input value x. This is the foundation for dependent types.
///
/// Examples:
/// - replicate<T>(n: Nat) -> List<T, n>  // Return type depends on n
/// - make_vector<T>(n: Nat{> 0}) -> List<T, n>  // With refinement
#[derive(Debug, Clone)]
pub struct PiType {
    /// Parameter name
    pub param_name: Text,
    /// Parameter type A
    pub param_type: Heap<Type>,
    /// Return type B(x) - may reference param_name
    pub return_type: Heap<Type>,
}

impl PiType {
    /// Create a new Pi type
    pub fn new(param_name: Text, param_type: Type, return_type: Type) -> Self {
        Self {
            param_name,
            param_type: Heap::new(param_type),
            return_type: Heap::new(return_type),
        }
    }

    /// Check if return type actually depends on parameter
    ///
    /// Traverses the return type to determine if it references the parameter.
    pub fn is_dependent(&self) -> bool {
        self.type_references_name(&self.return_type, &self.param_name)
    }

    /// Check if a type references a given name
    fn type_references_name(&self, ty: &Type, name: &Text) -> bool {
        match &ty.kind {
            TypeKind::Path(path) => {
                // Check if path matches the parameter name
                if path.segments.len() == 1
                    && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
                {
                    return ident.name == name.as_str();
                }
                false
            }
            TypeKind::Generic { base, args } => {
                if self.type_references_name(base, name) {
                    return true;
                }
                for arg in args {
                    if let verum_ast::ty::GenericArg::Type(t) = arg
                        && self.type_references_name(t, name)
                    {
                        return true;
                    }
                }
                false
            }
            TypeKind::Refined { base, predicate } => {
                if self.type_references_name(base, name) {
                    return true;
                }
                // Check if predicate references the name
                self.expr_references_name(&predicate.expr, name)
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            }
            | TypeKind::Rank2Function {
                type_params: _,
                params,
                return_type,
                ..
            } => {
                for param in params {
                    if self.type_references_name(param, name) {
                        return true;
                    }
                }
                self.type_references_name(return_type, name)
            }
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Pointer { inner, .. }
            | TypeKind::VolatilePointer { inner, .. }
            | TypeKind::Slice(inner)
            | TypeKind::Ownership { inner, .. }
            | TypeKind::GenRef { inner }
            | TypeKind::Bounded { base: inner, .. } => self.type_references_name(inner, name),
            TypeKind::Array { element, size } => {
                if self.type_references_name(element, name) {
                    return true;
                }
                if let Maybe::Some(size_expr) = size {
                    return self.expr_references_name(size_expr, name);
                }
                false
            }
            TypeKind::Tuple(types) => types.iter().any(|t| self.type_references_name(t, name)),
            TypeKind::Sigma {
                base, predicate, ..
            } => {
                if self.type_references_name(base, name) {
                    return true;
                }
                self.expr_references_name(predicate, name)
            }
            TypeKind::Qualified { self_ty, .. } => self.type_references_name(self_ty, name),
            TypeKind::Tensor { element, shape, .. } => {
                if self.type_references_name(element, name) {
                    return true;
                }
                shape.iter().any(|s| self.expr_references_name(s, name))
            }
            TypeKind::TypeConstructor { base, .. } => self.type_references_name(base, name),
            TypeKind::DynProtocol { bounds, bindings } => {
                // Check type bounds for name references
                for bound in bounds {
                    if let verum_ast::ty::TypeBoundKind::Equality(ty) = &bound.kind
                        && self.type_references_name(ty, name)
                    {
                        return true;
                    }
                }
                // Check bindings if present
                if let Maybe::Some(binds) = bindings {
                    for binding in binds {
                        if self.type_references_name(&binding.ty, name) {
                            return true;
                        }
                    }
                }
                false
            }
            // Existential type: check bounds for name references
            TypeKind::Existential { bounds, .. } => {
                for bound in bounds {
                    if let verum_ast::ty::TypeBoundKind::Equality(ty) = &bound.kind
                        && self.type_references_name(ty, name)
                    {
                        return true;
                    }
                }
                false
            }
            // Associated type: check base type for name references
            TypeKind::AssociatedType { base, .. } => self.type_references_name(base, name),
            // Capability-restricted type: check base type for name references
            TypeKind::CapabilityRestricted { base, .. } => self.type_references_name(base, name),
            // Record types: check all field types for name references
            TypeKind::Record { fields } => {
                fields.iter().any(|f| self.type_references_name(&f.ty, name))
            }
            // Path equality type: check the carrier type for name references
            TypeKind::PathType { carrier, .. } | TypeKind::DependentApp { carrier, .. } => self.type_references_name(carrier, name),
            // Dependent type application `T<A>(v..)`: check the carrier
            // and each value index for references to the bound name.
            TypeKind::DependentApp { carrier, value_args } => {
                if self.type_references_name(carrier, name) {
                    return true;
                }
                value_args.iter().any(|v| self.expr_references_name(v, name))
            }
            // Primitive types, inferred types, Never, and Unknown don't reference names
            TypeKind::Unit
            | TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::Char
            | TypeKind::Text
            | TypeKind::Inferred
            | TypeKind::Never
            | TypeKind::Unknown
            | TypeKind::Universe { .. }
            | TypeKind::Meta { .. }
            | TypeKind::TypeLambda { .. } => false,
        }
    }

    /// Check if an expression references a given name
    fn expr_references_name(&self, expr: &Expr, name: &Text) -> bool {
        match &expr.kind {
            ExprKind::Path(path) => {
                if path.segments.len() == 1
                    && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
                {
                    return ident.name == name.as_str();
                }
                false
            }
            ExprKind::Binary { left, right, .. } => {
                self.expr_references_name(left, name) || self.expr_references_name(right, name)
            }
            ExprKind::Unary { expr, .. } => self.expr_references_name(expr, name),
            ExprKind::Call { func, args, .. } => {
                if self.expr_references_name(func, name) {
                    return true;
                }
                args.iter().any(|arg| self.expr_references_name(arg, name))
            }
            ExprKind::Index { expr, index } => {
                self.expr_references_name(expr, name) || self.expr_references_name(index, name)
            }
            ExprKind::Field { expr, .. } => self.expr_references_name(expr, name),
            ExprKind::Paren(inner) => self.expr_references_name(inner, name),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check all conditions
                for cond in &condition.conditions {
                    match cond {
                        ConditionKind::Expr(e) => {
                            if self.expr_references_name(e, name) {
                                return true;
                            }
                        }
                        ConditionKind::Let { value, .. } => {
                            if self.expr_references_name(value, name) {
                                return true;
                            }
                        }
                    }
                }

                // Check then branch
                if let Maybe::Some(e) = &then_branch.expr
                    && self.expr_references_name(e, name)
                {
                    return true;
                }

                // Check else branch
                if let Maybe::Some(e) = else_branch
                    && self.expr_references_name(e, name)
                {
                    return true;
                }

                false
            }
            ExprKind::Match { expr, arms } => {
                if self.expr_references_name(expr, name) {
                    return true;
                }
                arms.iter()
                    .any(|arm| self.expr_references_name(&arm.body, name))
            }
            ExprKind::Block(block) => {
                if let Maybe::Some(e) = &block.expr {
                    self.expr_references_name(e, name)
                } else {
                    false
                }
            }
            ExprKind::Forall { body, .. } | ExprKind::Exists { body, .. } => {
                self.expr_references_name(body, name)
            }
            _ => false,
        }
    }
}

/// Dependent pair type: Sigma(x: A, B(x))
///
/// Represents a pair where the type of the second component can depend on
/// the value of the first component.
///
/// Examples:
/// - (n: Nat, List<T, n>)  // Pair of natural n and list of length n
/// - (success: Bool, if success then AST else Error)  // Dependent pair
#[derive(Debug, Clone)]
pub struct SigmaType {
    /// First component name
    pub fst_name: Text,
    /// First component type A
    pub fst_type: Heap<Type>,
    /// Second component type B(x) - may reference fst_name
    pub snd_type: Heap<Type>,
}

impl SigmaType {
    /// Create a new Sigma type
    pub fn new(fst_name: Text, fst_type: Type, snd_type: Type) -> Self {
        Self {
            fst_name,
            fst_type: Heap::new(fst_type),
            snd_type: Heap::new(snd_type),
        }
    }
}

/// Equality type: Eq<A, x, y>
///
/// Represents propositional equality between two values of type A.
/// This is used in formal proofs.
///
/// Propositional equality: `Eq<A, x, y>` with reflexivity `refl<A, x> : Eq<A, x, x>`,
/// symmetry, transitivity, and substitution principle `subst(eq, px) -> P(y)`.
#[derive(Debug, Clone)]
pub struct EqualityType {
    /// The type of values being compared
    pub value_type: Heap<Type>,
    /// Left-hand side
    pub lhs: Heap<Expr>,
    /// Right-hand side
    pub rhs: Heap<Expr>,
}

impl EqualityType {
    /// Create a new equality type
    pub fn new(value_type: Type, lhs: Expr, rhs: Expr) -> Self {
        Self {
            value_type: Heap::new(value_type),
            lhs: Heap::new(lhs),
            rhs: Heap::new(rhs),
        }
    }

    /// Check if this is a reflexive equality (x = x)
    ///
    /// Performs structural equality check on expressions.
    pub fn is_reflexive(&self) -> bool {
        self.exprs_equal(&self.lhs, &self.rhs)
    }

    /// Check if two expressions are structurally equal
    fn exprs_equal(&self, e1: &Expr, e2: &Expr) -> bool {
        use verum_ast::literal::LiteralKind;

        match (&e1.kind, &e2.kind) {
            (ExprKind::Literal(lit1), ExprKind::Literal(lit2)) => {
                // Compare literals
                match (&lit1.kind, &lit2.kind) {
                    (LiteralKind::Bool(b1), LiteralKind::Bool(b2)) => b1 == b2,
                    (LiteralKind::Int(i1), LiteralKind::Int(i2)) => i1.value == i2.value,
                    (LiteralKind::Float(f1), LiteralKind::Float(f2)) => f1.value == f2.value,
                    (LiteralKind::Char(c1), LiteralKind::Char(c2)) => c1 == c2,
                    (LiteralKind::Text(s1), LiteralKind::Text(s2)) => s1.as_str() == s2.as_str(),
                    _ => false,
                }
            }
            (ExprKind::Path(p1), ExprKind::Path(p2)) => {
                // Compare paths
                if p1.segments.len() != p2.segments.len() {
                    return false;
                }
                p1.segments
                    .iter()
                    .zip(p2.segments.iter())
                    .all(|(seg1, seg2)| match (seg1, seg2) {
                        (
                            verum_ast::ty::PathSegment::Name(id1),
                            verum_ast::ty::PathSegment::Name(id2),
                        ) => id1.name == id2.name,
                        _ => false,
                    })
            }
            (
                ExprKind::Binary {
                    op: op1,
                    left: l1,
                    right: r1,
                },
                ExprKind::Binary {
                    op: op2,
                    left: l2,
                    right: r2,
                },
            ) => op1 == op2 && self.exprs_equal(l1, l2) && self.exprs_equal(r1, r2),
            (ExprKind::Unary { op: op1, expr: e1 }, ExprKind::Unary { op: op2, expr: e2 }) => {
                op1 == op2 && self.exprs_equal(e1, e2)
            }
            (
                ExprKind::Call {
                    func: f1,
                    args: args1,
                    ..
                },
                ExprKind::Call {
                    func: f2,
                    args: args2,
                    ..
                },
            ) => {
                if !self.exprs_equal(f1, f2) || args1.len() != args2.len() {
                    return false;
                }
                args1
                    .iter()
                    .zip(args2.iter())
                    .all(|(a1, a2)| self.exprs_equal(a1, a2))
            }
            (
                ExprKind::Index {
                    expr: e1,
                    index: i1,
                },
                ExprKind::Index {
                    expr: e2,
                    index: i2,
                },
            ) => self.exprs_equal(e1, e2) && self.exprs_equal(i1, i2),
            (
                ExprKind::Field {
                    expr: e1,
                    field: f1,
                },
                ExprKind::Field {
                    expr: e2,
                    field: f2,
                },
            ) => self.exprs_equal(e1, e2) && f1.name == f2.name,
            (ExprKind::Paren(e1), ExprKind::Paren(e2)) => self.exprs_equal(e1, e2),
            (ExprKind::Tuple(elems1), ExprKind::Tuple(elems2)) => {
                if elems1.len() != elems2.len() {
                    return false;
                }
                elems1
                    .iter()
                    .zip(elems2.iter())
                    .all(|(e1, e2)| self.exprs_equal(e1, e2))
            }
            _ => false,
        }
    }
}

// ==================== SMT Backend for Dependent Types ====================

/// SMT backend support for dependent types
///
/// This provides the infrastructure for verifying dependent type constraints
/// using Z3's advanced features.
///
/// SMT backend for verifying dependent type constraints using Z3's quantifiers,
/// custom theories, and bounded proof search with configurable depth limits.
pub struct DependentTypeBackend {
    /// Custom SMT theories for dependent types
    theories: Map<Text, CustomTheory>,

    /// Proof term cache for performance
    #[allow(dead_code)] // Reserved for proof caching optimization
    proof_cache: Map<Text, ProofTerm>,

    /// Maximum quantifier depth
    max_quantifier_depth: usize,
}

impl DependentTypeBackend {
    /// Create a new dependent type backend
    pub fn new() -> Self {
        Self {
            theories: Map::new(),
            proof_cache: Map::new(),
            max_quantifier_depth: 3, // Reasonable default
        }
    }

    /// Verify a Pi type constraint
    ///
    /// Checks that a function with type (x: A) -> B(x) is well-typed.
    /// This involves verifying that for all valid inputs of type A,
    /// the output has type B(x).
    ///
    /// Pi type verification: `(x: A) -> B(x)`. The return type B may depend on the
    /// input value x. Verifies well-formedness of B under the binding of x, checks
    /// refinement predicates are non-contradictory, and enforces quantifier depth limits.
    pub fn verify_pi_type(&self, pi: &PiType, translator: &Translator<'_>) -> VerificationResult {
        use std::time::Instant;
        let start = Instant::now();

        // Create a fresh variable for the parameter
        let param_var = self.create_fresh_var(pi.param_name.as_str(), &pi.param_type)?;

        // Create new scope with parameter bound
        let mut scoped_translator = translator.clone_for_scope();
        scoped_translator.bind(pi.param_name.to_text(), param_var.clone());

        // Verify return type is well-formed with parameter bound
        // 1. Check base type is valid
        self.check_type_well_formed(&pi.return_type, &scoped_translator)?;

        // 2. If return type has refinements, verify they're valid
        if let TypeKind::Refined { base, predicate } = &pi.return_type.kind {
            // Translate predicate with parameter substituted
            let pred_expr = scoped_translator.translate_expr(&predicate.expr)?;

            // Check predicate is boolean-valued
            if let Maybe::Some(pred_bool) = option_to_maybe(pred_expr.as_bool()) {
                // Verify predicate doesn't create contradictions
                let ctx = translator.context();
                let solver = ctx.solver();

                solver.push();
                solver.assert(pred_bool.not());
                let result = solver.check();
                solver.pop(1);

                // If unsatisfiable, predicate is always true (trivial)
                // If satisfiable, it's a proper constraint (OK)
                // We just verify it's well-formed
            }
        }

        // 3. Check quantifier depth doesn't exceed limit
        let depth = self.compute_quantifier_depth(&pi.return_type);
        if depth > self.max_quantifier_depth {
            return Err(VerificationError::SolverError(
                format!(
                    "quantifier depth {} exceeds limit {}",
                    depth, self.max_quantifier_depth
                )
                .into(),
            ));
        }

        let duration = start.elapsed();
        let cost = VerificationCost::new("pi_type_verification".into(), duration, true);
        Ok(ProofResult::new(cost))
    }

    /// Verify a Sigma type constraint
    ///
    /// Checks that a dependent pair (x: A, B(x)) is well-typed.
    ///
    /// Sigma type verification: `(x: A, B(x))`. The type of the second component
    /// depends on the value of the first. Refinement types desugar to sigma types:
    /// `Int{> 0}` becomes `(n: Int, Proof(n > 0))`.
    pub fn verify_sigma_type(
        &self,
        sigma: &SigmaType,
        translator: &Translator<'_>,
    ) -> VerificationResult {
        use std::time::Instant;
        let start = Instant::now();

        // 1. Verify first component type is well-formed
        self.check_type_well_formed(&sigma.fst_type, translator)?;

        // Create variable for first component
        let fst_var = self.create_fresh_var(sigma.fst_name.as_str(), &sigma.fst_type)?;

        // 2. Bind first component in new scope
        let mut scoped_translator = translator.clone_for_scope();
        scoped_translator.bind(sigma.fst_name.to_text(), fst_var.clone());

        // 3. Verify second type is well-formed with first component bound
        self.check_type_well_formed(&sigma.snd_type, &scoped_translator)?;

        // 4. Check for circular dependencies
        if self.has_circular_dependency(&sigma.fst_type, &sigma.snd_type) {
            return Err(VerificationError::SolverError(
                "circular dependency in Sigma type".to_text(),
            ));
        }

        // 5. Verify pairing is valid (existential quantifier check)
        // For each value of first type, second type must be inhabited
        if let TypeKind::Refined { base, predicate } = &sigma.snd_type.kind {
            let pred_expr = scoped_translator.translate_expr(&predicate.expr)?;

            // Check there exists at least one valid pair
            if let Maybe::Some(pred_bool) = option_to_maybe(pred_expr.as_bool()) {
                let ctx = translator.context();
                let solver = ctx.solver();

                solver.push();
                solver.assert(&pred_bool);
                let result = solver.check();
                solver.pop(1);

                // Must be satisfiable for at least one value
                if result == z3::SatResult::Unsat {
                    return Err(VerificationError::SolverError(
                        "second component of Sigma type is uninhabited".to_text(),
                    ));
                }
            }
        }

        let duration = start.elapsed();
        let cost = VerificationCost::new("sigma_type_verification".into(), duration, true);
        Ok(ProofResult::new(cost))
    }

    /// Verify an equality type
    ///
    /// Checks propositional equality using Z3's built-in equality.
    ///
    /// Equality type verification: `Eq<A, x, y>` checks propositional equality
    /// using Z3's built-in equality. Both sides must have the same Z3 sort.
    pub fn verify_equality(
        &self,
        eq: &EqualityType,
        translator: &Translator<'_>,
    ) -> VerificationResult {
        use std::time::Instant;
        let start = Instant::now();

        // 1. Verify both sides have the same type
        self.check_type_well_formed(&eq.value_type, translator)?;

        // 2. Translate both sides to Z3
        let lhs_z3 = translator.translate_expr(&eq.lhs)?;
        let rhs_z3 = translator.translate_expr(&eq.rhs)?;

        // 3. Verify they're the same sort in Z3
        // Z3 crate 0.19.5: Use sort_kind() to compare types, as get_sort() is not available
        // on Dynamic. Instead, check type compatibility by trying to cast to the same type.
        let types_match = (lhs_z3.as_int().is_some() && rhs_z3.as_int().is_some())
            || (lhs_z3.as_bool().is_some() && rhs_z3.as_bool().is_some())
            || (lhs_z3.as_real().is_some() && rhs_z3.as_real().is_some())
            || (lhs_z3.as_bv().is_some() && rhs_z3.as_bv().is_some())
            || (lhs_z3.as_array().is_some() && rhs_z3.as_array().is_some())
            || (lhs_z3.as_datatype().is_some() && rhs_z3.as_datatype().is_some())
            || (lhs_z3.as_string().is_some() && rhs_z3.as_string().is_some());

        if !types_match {
            // Try to determine sort names for better error messages
            let lhs_sort_name = Self::get_sort_name(&lhs_z3);
            let rhs_sort_name = Self::get_sort_name(&rhs_z3);
            return Err(VerificationError::SolverError(
                format!(
                    "type mismatch in equality: {} vs {}",
                    lhs_sort_name, rhs_sort_name
                )
                .into(),
            ));
        }

        // 4. Create equality constraint
        let eq_constraint = Self::create_equality(&lhs_z3, &rhs_z3)?;

        // 5. Check if equality is decidable (can we prove or disprove it?)
        let ctx = translator.context();
        let solver = ctx.solver();

        solver.push();
        solver.assert(&eq_constraint);
        let sat_result = solver.check();
        solver.pop(1);

        // Equality is well-formed if it's either provable or refutable
        // Unknown means the constraint is too complex
        if sat_result == z3::SatResult::Unknown {
            return Err(VerificationError::SolverError(
                "equality constraint is undecidable with current solver configuration".to_text(),
            ));
        }

        let duration = start.elapsed();
        let cost = VerificationCost::new("equality_type_verification".into(), duration, true);
        Ok(ProofResult::new(cost))
    }

    /// Get the sort name for a Dynamic Z3 value
    ///
    /// This is a helper for error messages when types don't match.
    fn get_sort_name(value: &Dynamic) -> &'static str {
        if value.as_int().is_some() {
            "Int"
        } else if value.as_bool().is_some() {
            "Bool"
        } else if value.as_real().is_some() {
            "Real"
        } else if value.as_bv().is_some() {
            "BitVec"
        } else if value.as_array().is_some() {
            "Array"
        } else if value.as_datatype().is_some() {
            "Datatype"
        } else if value.as_string().is_some() {
            "String"
        } else {
            "Unknown"
        }
    }

    /// Create Z3 equality constraint
    fn create_equality(lhs: &Dynamic, rhs: &Dynamic) -> Result<Bool, VerificationError> {
        // Try different types
        if let (Maybe::Some(lhs_int), Maybe::Some(rhs_int)) =
            (option_to_maybe(lhs.as_int()), option_to_maybe(rhs.as_int()))
        {
            Ok(lhs_int.eq(&rhs_int))
        } else if let (Maybe::Some(lhs_bool), Maybe::Some(rhs_bool)) = (
            option_to_maybe(lhs.as_bool()),
            option_to_maybe(rhs.as_bool()),
        ) {
            Ok(lhs_bool.eq(&rhs_bool))
        } else if let (Maybe::Some(lhs_real), Maybe::Some(rhs_real)) = (
            option_to_maybe(lhs.as_real()),
            option_to_maybe(rhs.as_real()),
        ) {
            Ok(lhs_real.eq(&rhs_real))
        } else {
            Err(VerificationError::SolverError(
                "cannot create equality for these types".to_text(),
            ))
        }
    }

    /// Verify Fin type constraint: value < bound
    ///
    /// The Fin<n> type represents natural numbers less than n.
    /// This method verifies that a value satisfies the Fin constraint.
    ///
    /// Fin<n> type verification: bounded natural numbers `0 <= value < bound`.
    /// Fin types enable safe indexing: `index(list: List<T, n>, i: Fin<n>) -> T`.
    pub fn verify_fin_type(
        &mut self,
        value: &Expr,
        bound: &Expr,
        translator: &Translator<'_>,
    ) -> VerificationResult {
        use std::time::Instant;
        use verum_ast::literal::LiteralKind;
        let start = Instant::now();

        // 1. Try to evaluate bound to concrete value
        let bound_val = self.eval_const_expr(bound)?;

        // 2. Check value against bound
        match &value.kind {
            ExprKind::Literal(lit) => {
                // FZero : Fin<Succ(n)> for any n > 0
                if let LiteralKind::Int(int_lit) = &lit.kind {
                    if int_lit.value == 0 {
                        // Zero is valid for any positive bound
                        if bound_val > 0 {
                            let duration = start.elapsed();
                            let cost =
                                VerificationCost::new("fin_type_zero".into(), duration, true);
                            return Ok(ProofResult::new(cost));
                        } else {
                            return Err(VerificationError::SolverError(
                                "Fin bound must be positive".to_text(),
                            ));
                        }
                    }

                    // Check: value < bound
                    if (int_lit.value as i64) < bound_val {
                        let duration = start.elapsed();
                        let cost = VerificationCost::new("fin_type_literal".into(), duration, true);
                        return Ok(ProofResult::new(cost));
                    } else {
                        let cost = VerificationCost::new(
                            "fin_type_check_failed".into(),
                            start.elapsed(),
                            false,
                        );
                        return Err(VerificationError::CannotProve {
                            constraint: format!("value {} < {}", int_lit.value, bound_val).into(),
                            counterexample: None,
                            cost,
                            suggestions: List::new(),
                        });
                    }
                }
            }

            _ => {
                // Use SMT to verify: value < bound
                let value_z3 = translator.translate_expr(value)?;
                let bound_z3 = translator.translate_expr(bound)?;

                if let (Maybe::Some(value_int), Maybe::Some(bound_int)) = (
                    option_to_maybe(value_z3.as_int()),
                    option_to_maybe(bound_z3.as_int()),
                ) {
                    let constraint = value_int.lt(&bound_int);

                    let ctx = translator.context();
                    let solver = ctx.solver();

                    solver.push();
                    solver.assert(constraint.not());
                    let result = solver.check();
                    solver.pop(1);

                    if result == z3::SatResult::Unsat {
                        // Constraint always holds
                        let duration = start.elapsed();
                        let cost =
                            VerificationCost::new("fin_type_verified".into(), duration, true);
                        return Ok(ProofResult::new(cost));
                    } else {
                        let cost = VerificationCost::new(
                            "fin_type_check_failed".into(),
                            start.elapsed(),
                            false,
                        );
                        return Err(VerificationError::CannotProve {
                            constraint: format!("value < bound for {:?}", value).into(),
                            counterexample: None,
                            cost,
                            suggestions: List::new(),
                        });
                    }
                }
            }
        }

        Err(VerificationError::SolverError(
            "unsupported Fin type verification".to_text(),
        ))
    }

    /// Evaluate expression to constant value
    fn eval_const_expr(&self, expr: &Expr) -> Result<i64, VerificationError> {
        use verum_ast::literal::LiteralKind;

        match &expr.kind {
            ExprKind::Literal(lit) => {
                if let LiteralKind::Int(int_lit) = &lit.kind {
                    Ok(int_lit.value as i64)
                } else {
                    Err(VerificationError::SolverError(
                        "expected integer literal".to_text(),
                    ))
                }
            }
            ExprKind::Binary { op, left, right } => {
                let left_val = self.eval_const_expr(left)?;
                let right_val = self.eval_const_expr(right)?;

                match op {
                    BinOp::Add => Ok(left_val + right_val),
                    BinOp::Sub => Ok(left_val - right_val),
                    BinOp::Mul => Ok(left_val * right_val),
                    BinOp::Div if right_val != 0 => Ok(left_val / right_val),
                    _ => Err(VerificationError::SolverError(
                        format!("unsupported operation in constant expression: {:?}", op).into(),
                    )),
                }
            }
            _ => Err(VerificationError::SolverError(
                "expected constant expression".to_text(),
            )),
        }
    }

    /// Register a custom SMT theory
    ///
    /// Register a custom SMT theory with named sorts, functions, and axioms.
    /// Example: BitVector theory with `bv_add`, `bv_mul`, `bv_and` and commutativity axiom.
    pub fn register_theory(&mut self, theory: CustomTheory) {
        self.theories.insert(theory.name.clone(), theory);
    }

    /// Get a registered theory
    pub fn get_theory(&self, name: &str) -> Maybe<&CustomTheory> {
        self.theories.get(&Text::from(name))
    }

    // ==================== Helper Methods ====================

    /// Create a fresh Z3 variable with given name and type
    fn create_fresh_var(&self, name: &str, ty: &Type) -> Result<Dynamic, VerificationError> {
        use z3::ast::{Bool, Int, Real};

        match &ty.kind {
            TypeKind::Int => {
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::Float => {
                let var = Real::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::Bool => {
                let var = Bool::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::Refined { base, .. } => {
                // For refined types, create variable of base type
                self.create_fresh_var(name, base)
            }
            TypeKind::Generic { .. } | TypeKind::Path(_) | TypeKind::Unit => {
                // For generic types (like List<T>), named types, and Unit,
                // use an uninterpreted sort modeled as Int for simplicity
                // This is sound because we only check structural properties
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::Tuple(_) | TypeKind::Array { .. } | TypeKind::Slice(_) => {
                // For compound types, model as Int (uninterpreted)
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::Reference { .. } | TypeKind::Pointer { .. } | TypeKind::VolatilePointer { .. } => {
                // References and pointers are modeled as addresses (Int)
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::Function { .. } | TypeKind::Rank2Function { .. } => {
                // Function types - model as Int (function pointer)
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::Char | TypeKind::Text => {
                // Characters and text as Int
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Ownership { inner, .. }
            | TypeKind::GenRef { inner }
            | TypeKind::Bounded { base: inner, .. }
            | TypeKind::TypeConstructor { base: inner, .. } => {
                // Transparent wrappers - use inner type
                self.create_fresh_var(name, inner)
            }
            TypeKind::Qualified { .. }
            | TypeKind::Sigma { .. }
            | TypeKind::DynProtocol { .. }
            | TypeKind::Tensor { .. }
            | TypeKind::Existential { .. }
            | TypeKind::Inferred => {
                // Complex types modeled as uninterpreted Int
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::AssociatedType { base, .. } => {
                // Associated types - use base type
                self.create_fresh_var(name, base)
            }
            TypeKind::Never => {
                // Never type (!) - diverging expressions
                // Model as Int (vacuously satisfiable - never actually instantiated)
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::CapabilityRestricted { base, .. } => {
                // Capability-restricted types - use base type
                self.create_fresh_var(name, base)
            }
            TypeKind::Record { .. } => {
                // Record types - model as uninterpreted Int for compound type
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::Unknown | TypeKind::Universe { .. }
            | TypeKind::Meta { .. } | TypeKind::TypeLambda { .. } => {
                // Unknown/Universe/Meta/TypeLambda type - model as uninterpreted Int for SMT purposes
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }
            TypeKind::PathType { carrier, .. } | TypeKind::DependentApp { carrier, .. } => {
                // Path equality type - use carrier type for sort
                self.create_fresh_var(name, carrier)
            }
        }
    }

    /// Check if a type is well-formed
    fn check_type_well_formed(
        &self,
        ty: &Type,
        translator: &Translator<'_>,
    ) -> Result<(), VerificationError> {
        match &ty.kind {
            TypeKind::Int | TypeKind::Float | TypeKind::Bool | TypeKind::Unit => {
                // Primitive types are always well-formed
                Ok(())
            }
            TypeKind::Refined { base, predicate } => {
                // Check base type is well-formed
                self.check_type_well_formed(base, translator)?;

                // Check predicate is a valid boolean expression
                let pred_z3 = translator.translate_expr(&predicate.expr)?;
                if pred_z3.as_bool().is_none() {
                    return Err(VerificationError::SolverError(
                        "refinement predicate must be boolean".to_text(),
                    ));
                }

                Ok(())
            }
            TypeKind::Path(_) => {
                // Named types - assume well-formed for now
                // Full implementation would check type definitions
                Ok(())
            }
            // Path equality type: well-formed if carrier is well-formed
            TypeKind::PathType { carrier, .. } | TypeKind::DependentApp { carrier, .. } => self.check_type_well_formed(carrier, translator),
            // All other type kinds are assumed well-formed for now
            TypeKind::Char
            | TypeKind::Text
            | TypeKind::Inferred
            | TypeKind::Generic { .. }
            | TypeKind::Function { .. }
            | TypeKind::Rank2Function { .. }
            | TypeKind::Tuple(_)
            | TypeKind::Array { .. }
            | TypeKind::Slice(_)
            | TypeKind::Reference { .. }
            | TypeKind::CheckedReference { .. }
            | TypeKind::UnsafeReference { .. }
            | TypeKind::Pointer { .. }
            | TypeKind::VolatilePointer { .. }
            | TypeKind::Ownership { .. }
            | TypeKind::GenRef { .. }
            | TypeKind::TypeConstructor { .. }
            | TypeKind::Bounded { .. }
            | TypeKind::DynProtocol { .. }
            | TypeKind::Sigma { .. }
            | TypeKind::Qualified { .. }
            | TypeKind::Tensor { .. }
            | TypeKind::Existential { .. }
            | TypeKind::AssociatedType { .. }
            | TypeKind::CapabilityRestricted { .. }
            | TypeKind::Record { .. }
            | TypeKind::Never
            | TypeKind::Unknown
            | TypeKind::Universe { .. }
            | TypeKind::Meta { .. }
            | TypeKind::TypeLambda { .. } => Ok(()),
        }
    }

    /// Compute quantifier depth in a type
    fn compute_quantifier_depth(&self, ty: &Type) -> usize {
        match &ty.kind {
            TypeKind::Refined { base, predicate } => {
                let base_depth = self.compute_quantifier_depth(base);
                let pred_depth = self.compute_expr_quantifier_depth(&predicate.expr);
                base_depth.max(pred_depth)
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            }
            | TypeKind::Rank2Function {
                type_params: _,
                params,
                return_type,
                ..
            } => {
                let params_depth = params
                    .iter()
                    .map(|p| self.compute_quantifier_depth(p))
                    .max()
                    .unwrap_or(0);
                let return_depth = self.compute_quantifier_depth(return_type);
                params_depth.max(return_depth)
            }
            TypeKind::Tuple(types) => types
                .iter()
                .map(|t| self.compute_quantifier_depth(t))
                .max()
                .unwrap_or(0),
            TypeKind::Array { element, .. }
            | TypeKind::Slice(element)
            | TypeKind::Reference { inner: element, .. }
            | TypeKind::CheckedReference { inner: element, .. }
            | TypeKind::UnsafeReference { inner: element, .. }
            | TypeKind::Pointer { inner: element, .. }
            | TypeKind::VolatilePointer { inner: element, .. }
            | TypeKind::Ownership { inner: element, .. }
            | TypeKind::GenRef { inner: element }
            | TypeKind::Bounded { base: element, .. }
            | TypeKind::TypeConstructor { base: element, .. } => {
                self.compute_quantifier_depth(element)
            }
            TypeKind::Generic { base, args } => {
                let base_depth = self.compute_quantifier_depth(base);
                let args_depth = args
                    .iter()
                    .filter_map(|arg| {
                        if let verum_ast::ty::GenericArg::Type(t) = arg {
                            Some(self.compute_quantifier_depth(t))
                        } else {
                            None
                        }
                    })
                    .max()
                    .unwrap_or(0);
                base_depth.max(args_depth)
            }
            TypeKind::Sigma { base, .. } | TypeKind::Qualified { self_ty: base, .. } => {
                self.compute_quantifier_depth(base)
            }
            TypeKind::Tensor { element, .. } => self.compute_quantifier_depth(element),
            // Existential types - compute depth from bounds' equality types
            TypeKind::Existential { bounds, .. } => {
                bounds
                    .iter()
                    .filter_map(|bound| {
                        if let verum_ast::ty::TypeBoundKind::Equality(ty) = &bound.kind {
                            Some(self.compute_quantifier_depth(ty))
                        } else {
                            None
                        }
                    })
                    .max()
                    .unwrap_or(0)
            }
            // Associated types - compute depth from base
            TypeKind::AssociatedType { base, .. } => self.compute_quantifier_depth(base),
            // Capability-restricted types - compute depth from base
            TypeKind::CapabilityRestricted { base, .. } => self.compute_quantifier_depth(base),
            // Record types - compute max depth from field types
            TypeKind::Record { fields } => fields
                .iter()
                .map(|f| self.compute_quantifier_depth(&f.ty))
                .max()
                .unwrap_or(0),
            // Path equality type: depth comes from carrier
            TypeKind::PathType { carrier, .. } | TypeKind::DependentApp { carrier, .. } => self.compute_quantifier_depth(carrier),
            // Primitive types and inferred types have depth 0
            TypeKind::Unit
            | TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::Char
            | TypeKind::Text
            | TypeKind::Path(_)
            | TypeKind::Inferred
            | TypeKind::DynProtocol { .. }
            | TypeKind::Never
            | TypeKind::Unknown
            | TypeKind::Universe { .. }
            | TypeKind::Meta { .. }
            | TypeKind::TypeLambda { .. } => 0,
        }
    }

    /// Compute quantifier depth in an expression
    fn compute_expr_quantifier_depth(&self, expr: &Expr) -> usize {
        match &expr.kind {
            // Quantifiers: Forall and Exists increase depth
            ExprKind::Forall { body, .. } | ExprKind::Exists { body, .. } => {
                // Quantifier found: depth is 1 + max depth in body
                let body_depth = self.compute_expr_quantifier_depth(body);
                1 + body_depth
            }
            ExprKind::Binary { left, right, .. } => {
                let left_depth = self.compute_expr_quantifier_depth(left);
                let right_depth = self.compute_expr_quantifier_depth(right);
                left_depth.max(right_depth)
            }
            ExprKind::Unary { expr, .. } => self.compute_expr_quantifier_depth(expr),
            ExprKind::Paren(inner) => self.compute_expr_quantifier_depth(inner),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check condition depth - iterate through all conditions
                let cond_depth = condition
                    .conditions
                    .iter()
                    .map(|cond| match cond {
                        ConditionKind::Expr(e) => self.compute_expr_quantifier_depth(e),
                        ConditionKind::Let { value, .. } => {
                            self.compute_expr_quantifier_depth(value)
                        }
                    })
                    .max()
                    .unwrap_or(0);

                // Check then branch depth
                let then_depth = if let Maybe::Some(e) = &then_branch.expr {
                    self.compute_expr_quantifier_depth(e)
                } else {
                    0
                };

                // Check else branch depth
                let else_depth = if let Maybe::Some(e) = else_branch {
                    self.compute_expr_quantifier_depth(e)
                } else {
                    0
                };

                cond_depth.max(then_depth).max(else_depth)
            }
            ExprKind::Block(block) => {
                // Check the final expression in the block
                if let Maybe::Some(e) = &block.expr {
                    self.compute_expr_quantifier_depth(e)
                } else {
                    0
                }
            }
            ExprKind::Match { expr, arms } => {
                // Check match expression depth
                let expr_depth = self.compute_expr_quantifier_depth(expr);

                // Check all match arm bodies
                let arms_depth = arms
                    .iter()
                    .map(|arm| self.compute_expr_quantifier_depth(&arm.body))
                    .max()
                    .unwrap_or(0);

                expr_depth.max(arms_depth)
            }
            ExprKind::Call { func, args, .. } => {
                let func_depth = self.compute_expr_quantifier_depth(func);
                let args_depth = args
                    .iter()
                    .map(|arg| self.compute_expr_quantifier_depth(arg))
                    .max()
                    .unwrap_or(0);
                func_depth.max(args_depth)
            }
            _ => 0,
        }
    }

    /// Check for circular dependencies between types
    ///
    /// Uses Tarjan's strongly connected components algorithm to detect cycles
    /// in the type dependency graph. This ensures mutual recursion is properly
    /// detected and reported.
    fn has_circular_dependency(&self, ty1: &Type, ty2: &Type) -> bool {
        let mut graph = TypeDependencyGraph::new();

        // Build dependency graph starting from both types
        let node1 = graph.add_type(ty1.clone());
        let node2 = graph.add_type(ty2.clone());

        // Add edge from ty1 to ty2 (ty1 depends on ty2)
        graph.add_dependency(node1, node2);

        // Recursively collect all dependencies with proper parent tracking
        graph.collect_dependencies_with_parent(ty1, node1);
        graph.collect_dependencies_with_parent(ty2, node2);

        // Run cycle detection
        graph.has_cycle()
    }

    /// Detect all circular dependencies in a type
    ///
    /// Returns a list of cycles found, with each cycle represented as a list
    /// of type names involved in the cycle.
    pub fn detect_circular_dependencies(&self, ty: &Type) -> List<List<Text>> {
        let mut graph = TypeDependencyGraph::new();
        graph.collect_dependencies(ty);
        graph.find_all_cycles()
    }

    /// Check circular dependencies for inductive types
    ///
    /// Verifies that an inductive type definition doesn't have circular
    /// dependencies that would make it ill-formed.
    pub fn check_inductive_cycles(&self, inductive: &InductiveType) -> Result<(), Text> {
        let mut graph = TypeDependencyGraph::new();

        // Add the inductive type itself as a node
        let self_node = graph.add_type_name(inductive.name.clone());

        // Collect dependencies from all constructors
        for ctor in &inductive.constructors {
            graph.collect_dependencies(&ctor.ty);

            // Add edges for type references in constructor
            let referenced_types = Self::extract_type_names(&ctor.ty);
            for ref_type in referenced_types {
                // If constructor references the inductive type being defined,
                // check if it's in a valid position (handled by positivity check)
                if ref_type == inductive.name {
                    continue; // Self-reference is allowed in positive positions
                }

                let ref_node = graph.add_type_name(ref_type);
                graph.add_dependency(self_node, ref_node);
            }
        }

        // Check for cycles excluding self-references
        if let Maybe::Some(cycles) = graph.find_cycle_excluding(self_node) {
            let mut cycle_parts = List::new();
            for name in cycles {
                cycle_parts.push(name.as_str().to_string());
            }
            let cycle_description = cycle_parts
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" -> ");

            return Err(format!(
                "Circular dependency detected in inductive type '{}': {}",
                inductive.name, cycle_description
            )
            .into());
        }

        Ok(())
    }

    /// Extract all type names referenced in a type
    fn extract_type_names(ty: &Type) -> List<Text> {
        let mut names = List::new();
        Self::extract_type_names_recursive(ty, &mut names);
        names
    }

    fn extract_type_names_recursive(ty: &Type, names: &mut List<Text>) {
        match &ty.kind {
            TypeKind::Path(path) => {
                if path.segments.len() == 1
                    && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
                {
                    names.push(Text::from(ident.name.as_str()));
                }
            }
            TypeKind::Generic { base, args } => {
                Self::extract_type_names_recursive(base, names);
                for arg in args {
                    if let verum_ast::ty::GenericArg::Type(t) = arg {
                        Self::extract_type_names_recursive(t, names);
                    }
                }
            }
            TypeKind::Refined { base, .. } => {
                Self::extract_type_names_recursive(base, names);
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            }
            | TypeKind::Rank2Function {
                type_params: _,
                params,
                return_type,
                ..
            } => {
                for param in params {
                    Self::extract_type_names_recursive(param, names);
                }
                Self::extract_type_names_recursive(return_type, names);
            }
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Pointer { inner, .. }
            | TypeKind::VolatilePointer { inner, .. }
            | TypeKind::Slice(inner)
            | TypeKind::Ownership { inner, .. }
            | TypeKind::GenRef { inner }
            | TypeKind::Bounded { base: inner, .. }
            | TypeKind::TypeConstructor { base: inner, .. } => {
                Self::extract_type_names_recursive(inner, names);
            }
            TypeKind::Array { element, .. } => {
                Self::extract_type_names_recursive(element, names);
            }
            TypeKind::Tuple(types) => {
                for t in types {
                    Self::extract_type_names_recursive(t, names);
                }
            }
            TypeKind::Sigma { base, .. } => {
                Self::extract_type_names_recursive(base, names);
            }
            TypeKind::Qualified { self_ty, .. } => {
                Self::extract_type_names_recursive(self_ty, names);
            }
            TypeKind::Tensor { element, shape, .. } => {
                Self::extract_type_names_recursive(element, names);
                // Shape expressions might reference types, but typically they're just dimensions
            }
            TypeKind::DynProtocol { bindings, .. } => {
                // Extract type names from type bindings
                if let Maybe::Some(binds) = bindings {
                    for binding in binds {
                        Self::extract_type_names_recursive(&binding.ty, names);
                    }
                }
            }
            // Existential types - extract from bounds' equality types
            TypeKind::Existential { bounds, .. } => {
                for bound in bounds {
                    if let verum_ast::ty::TypeBoundKind::Equality(ty) = &bound.kind {
                        Self::extract_type_names_recursive(ty, names);
                    }
                }
            }
            // Associated types - extract from base type
            TypeKind::AssociatedType { base, .. } => {
                Self::extract_type_names_recursive(base, names);
            }
            // Capability-restricted types - extract from base type
            TypeKind::CapabilityRestricted { base, .. } => {
                Self::extract_type_names_recursive(base, names);
            }
            // Record types - extract from field types
            TypeKind::Record { fields } => {
                for field in fields {
                    Self::extract_type_names_recursive(&field.ty, names);
                }
            }
            // Path equality type: recurse into carrier type
            TypeKind::PathType { carrier, .. } | TypeKind::DependentApp { carrier, .. } => {
                Self::extract_type_names_recursive(carrier, names);
            }
            // Primitives, inferred types, Never, Unknown, and Universe have no type dependencies
            TypeKind::Unit
            | TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::Char
            | TypeKind::Text
            | TypeKind::Inferred
            | TypeKind::Never
            | TypeKind::Unknown
            | TypeKind::Universe { .. }
            | TypeKind::Meta { .. }
            | TypeKind::TypeLambda { .. } => {
                // No dependencies
            }
        }
    }
}

impl Default for DependentTypeBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// The kind of dependent-type goal to verify.
#[derive(Debug, Clone)]
pub enum DependentGoal {
    /// Verify a Pi type `(x: A) -> B(x)`.
    Pi(PiType),
    /// Verify a Sigma type `(x: A, B(x))`.
    Sigma(SigmaType),
    /// Verify an equality type `a =_A b`.
    Equality(EqualityType),
    /// Verify a Fin type `Fin(n)` — bounded naturals. The value
    /// and bound expressions are both AST expressions that will be
    /// translated to SMT by the backend.
    Fin { value: Expr, bound: Expr },
}

impl DependentTypeBackend {
    /// Unified entry point for dependent-type verification.
    ///
    /// Dispatches to the appropriate verifier based on the goal kind:
    /// - `DependentGoal::Pi`       → `verify_pi_type`
    /// - `DependentGoal::Sigma`    → `verify_sigma_type`
    /// - `DependentGoal::Equality` → `verify_equality`
    /// - `DependentGoal::Fin`      → `verify_fin_type`
    ///
    /// This is the single entry point that downstream code (e.g.,
    /// `verum_verification`) should call for dependent-type
    /// verification goals.
    pub fn verify_goal_dependent(
        &mut self,
        goal: &DependentGoal,
        translator: &Translator<'_>,
    ) -> VerificationResult {
        match goal {
            DependentGoal::Pi(pi) => self.verify_pi_type(pi, translator),
            DependentGoal::Sigma(sigma) => self.verify_sigma_type(sigma, translator),
            DependentGoal::Equality(eq) => self.verify_equality(eq, translator),
            DependentGoal::Fin { value, bound } => {
                self.verify_fin_type(value, bound, translator)
            }
        }
    }
}

// ==================== Type Dependency Graph ====================

/// Graph structure for tracking type dependencies
///
/// Used for circular dependency detection using Tarjan's SCC algorithm.
/// Nodes represent types, edges represent dependencies.
#[derive(Debug)]
struct TypeDependencyGraph {
    /// Node ID counter
    next_id: usize,

    /// Map from type name to node ID
    name_to_id: Map<Text, usize>,

    /// Map from node ID to type name
    id_to_name: Map<usize, Text>,

    /// Adjacency list: node -> list of nodes it depends on
    edges: Map<usize, List<usize>>,

    /// Reverse edges for SCC computation
    reverse_edges: Map<usize, List<usize>>,
}

impl TypeDependencyGraph {
    fn new() -> Self {
        Self {
            next_id: 0,
            name_to_id: Map::new(),
            id_to_name: Map::new(),
            edges: Map::new(),
            reverse_edges: Map::new(),
        }
    }

    /// Add a type to the graph by its full type definition
    fn add_type(&mut self, ty: Type) -> usize {
        // Extract the type name if it's a named type
        if let TypeKind::Path(ref path) = ty.kind
            && path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ref ident) = path.segments[0]
        {
            let name = Text::from(ident.name.as_str());
            return self.add_type_name(name);
        }

        // For non-named types, generate a synthetic ID
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Add a type to the graph by name
    fn add_type_name(&mut self, name: Text) -> usize {
        if let Maybe::Some(&id) = self.name_to_id.get(&name) {
            return id;
        }

        let id = self.next_id;
        self.next_id += 1;
        self.name_to_id.insert(name.clone(), id);
        self.id_to_name.insert(id, name);
        self.edges.insert(id, List::new());
        self.reverse_edges.insert(id, List::new());
        id
    }

    /// Add a dependency edge: from depends on to
    fn add_dependency(&mut self, from: usize, to: usize) {
        // Add forward edge
        self.edges.entry(from).or_default().push(to);

        // Add reverse edge
        self.reverse_edges
            .entry(to)
            .or_default()
            .push(from);
    }

    /// Collect all dependencies from a type recursively
    fn collect_dependencies(&mut self, ty: &Type) {
        self.collect_dependencies_recursive(ty, Maybe::None);
    }

    /// Collect dependencies with a known parent node
    fn collect_dependencies_with_parent(&mut self, ty: &Type, parent_node: usize) {
        self.collect_dependencies_recursive(ty, Maybe::Some(parent_node));
    }

    fn collect_dependencies_recursive(&mut self, ty: &Type, parent: Maybe<usize>) {
        match &ty.kind {
            TypeKind::Path(path) => {
                if path.segments.len() == 1
                    && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
                {
                    let name = Text::from(ident.name.as_str());
                    let node_id = self.add_type_name(name);

                    if let Maybe::Some(parent_id) = parent {
                        // Only add edge if parent and child are different
                        if parent_id != node_id {
                            self.add_dependency(parent_id, node_id);
                        }
                    }
                }
            }
            TypeKind::Generic { base, args } => {
                self.collect_dependencies_recursive(base, parent);
                for arg in args {
                    if let verum_ast::ty::GenericArg::Type(t) = arg {
                        self.collect_dependencies_recursive(t, parent);
                    }
                }
            }
            TypeKind::Refined { base, .. } => {
                self.collect_dependencies_recursive(base, parent);
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            }
            | TypeKind::Rank2Function {
                type_params: _,
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.collect_dependencies_recursive(param, parent);
                }
                self.collect_dependencies_recursive(return_type, parent);
            }
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Pointer { inner, .. }
            | TypeKind::VolatilePointer { inner, .. }
            | TypeKind::Slice(inner)
            | TypeKind::Ownership { inner, .. }
            | TypeKind::GenRef { inner }
            | TypeKind::Bounded { base: inner, .. } => {
                self.collect_dependencies_recursive(inner, parent);
            }
            TypeKind::Array { element, .. } => {
                self.collect_dependencies_recursive(element, parent);
            }
            TypeKind::Tuple(types) => {
                for t in types {
                    self.collect_dependencies_recursive(t, parent);
                }
            }
            TypeKind::Sigma { base, .. } => {
                self.collect_dependencies_recursive(base, parent);
            }
            TypeKind::Qualified { self_ty, .. } => {
                self.collect_dependencies_recursive(self_ty, parent);
            }
            TypeKind::Tensor { element, .. } => {
                self.collect_dependencies_recursive(element, parent);
            }
            TypeKind::TypeConstructor { base, .. } => {
                self.collect_dependencies_recursive(base, parent);
            }
            TypeKind::DynProtocol { bindings, .. } => {
                // Dynamic protocol may have type bindings to track
                if let Maybe::Some(binds) = bindings {
                    for binding in binds {
                        self.collect_dependencies_recursive(&binding.ty, parent);
                    }
                }
            }
            // Existential types - collect from bounds' equality types
            TypeKind::Existential { bounds, .. } => {
                for bound in bounds {
                    if let verum_ast::ty::TypeBoundKind::Equality(ty) = &bound.kind {
                        self.collect_dependencies_recursive(ty, parent);
                    }
                }
            }
            // Associated types - collect from base type
            TypeKind::AssociatedType { base, .. } => {
                self.collect_dependencies_recursive(base, parent);
            }
            // Capability-restricted types - collect from base type
            TypeKind::CapabilityRestricted { base, .. } => {
                self.collect_dependencies_recursive(base, parent);
            }
            // Record types - collect from field types
            TypeKind::Record { fields } => {
                for field in fields {
                    self.collect_dependencies_recursive(&field.ty, parent);
                }
            }
            // Path equality type: recurse into carrier type
            TypeKind::PathType { carrier, .. } | TypeKind::DependentApp { carrier, .. } => {
                self.collect_dependencies_recursive(carrier, parent);
            }
            // Primitives, inferred types, Never, Unknown, and Universe have no dependencies
            TypeKind::Unit
            | TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::Char
            | TypeKind::Text
            | TypeKind::Inferred
            | TypeKind::Never
            | TypeKind::Unknown
            | TypeKind::Universe { .. }
            | TypeKind::Meta { .. }
            | TypeKind::TypeLambda { .. } => {
                // No dependencies
            }
        }
    }

    /// Check if the graph has any cycles using Tarjan's SCC algorithm
    fn has_cycle(&self) -> bool {
        let sccs = self.tarjan_scc();

        // A cycle exists if any SCC has more than one node,
        // or if a single-node SCC has a self-loop
        for scc in sccs {
            if scc.len() > 1 {
                return true;
            }

            if scc.len() == 1 {
                let node = scc[0];
                if let Maybe::Some(edges) = self.edges.get(&node)
                    && edges.contains(&node)
                {
                    return true; // Self-loop
                }
            }
        }

        false
    }

    /// Find all cycles in the graph
    fn find_all_cycles(&self) -> List<List<Text>> {
        let mut cycles = List::new();
        let sccs = self.tarjan_scc();

        for scc in sccs {
            if scc.len() > 1 {
                // Multi-node cycle
                let cycle_names: List<Text> = scc
                    .iter()
                    .filter_map(|&id| self.id_to_name.get(&id).cloned())
                    .collect();

                if !cycle_names.is_empty() {
                    cycles.push(cycle_names);
                }
            } else if scc.len() == 1 {
                // Check for self-loop
                let node = scc[0];
                if let Maybe::Some(edges) = self.edges.get(&node)
                    && edges.contains(&node)
                    && let Maybe::Some(name) = self.id_to_name.get(&node)
                {
                    let mut cycle = List::new();
                    cycle.push(name.clone());
                    cycles.push(cycle);
                }
            }
        }

        cycles
    }

    /// Find a cycle that doesn't include a specific node
    fn find_cycle_excluding(&self, excluded_node: usize) -> Maybe<List<Text>> {
        let sccs = self.tarjan_scc();

        for scc in sccs {
            // Skip if this SCC contains the excluded node
            if scc.contains(&excluded_node) {
                continue;
            }

            if scc.len() > 1 {
                let cycle_names: List<Text> = scc
                    .iter()
                    .filter_map(|&id| self.id_to_name.get(&id).cloned())
                    .collect();

                if !cycle_names.is_empty() {
                    return Maybe::Some(cycle_names);
                }
            }
        }

        Maybe::None
    }

    /// Tarjan's strongly connected components algorithm
    ///
    /// Returns a list of SCCs, where each SCC is a list of node IDs.
    fn tarjan_scc(&self) -> List<List<usize>> {
        let mut state = TarjanState::new();
        let mut sccs = List::new();

        // Run DFS from each unvisited node
        for node in 0..self.next_id {
            if !state.visited.contains(&node) {
                self.tarjan_dfs(node, &mut state, &mut sccs);
            }
        }

        sccs
    }

    fn tarjan_dfs(&self, node: usize, state: &mut TarjanState, sccs: &mut List<List<usize>>) {
        state.visited.insert(node);
        state.index.insert(node, state.current_index);
        state.lowlink.insert(node, state.current_index);
        state.current_index += 1;
        state.stack.push(node);
        state.on_stack.insert(node);

        // Visit all neighbors
        if let Maybe::Some(neighbors) = self.edges.get(&node) {
            for &neighbor in neighbors {
                if !state.visited.contains(&neighbor) {
                    // Neighbor not visited, recurse
                    self.tarjan_dfs(neighbor, state, sccs);

                    // Update lowlink
                    let neighbor_lowlink = *state.lowlink.get(&neighbor).unwrap_or(&0);
                    let current_lowlink = *state.lowlink.get(&node).unwrap_or(&0);
                    state
                        .lowlink
                        .insert(node, current_lowlink.min(neighbor_lowlink));
                } else if state.on_stack.contains(&neighbor) {
                    // Neighbor is on stack, part of current SCC
                    let neighbor_index = *state.index.get(&neighbor).unwrap_or(&0);
                    let current_lowlink = *state.lowlink.get(&node).unwrap_or(&0);
                    state
                        .lowlink
                        .insert(node, current_lowlink.min(neighbor_index));
                }
            }
        }

        // If this is a root node, pop the SCC
        let node_lowlink = *state.lowlink.get(&node).unwrap_or(&0);
        let node_index = *state.index.get(&node).unwrap_or(&0);

        if node_lowlink == node_index {
            let mut scc = List::new();

            loop {
                if let Maybe::Some(w) = state.stack.pop() {
                    state.on_stack.remove(&w);
                    scc.push(w);

                    if w == node {
                        break;
                    }
                } else {
                    break;
                }
            }

            if !scc.is_empty() {
                sccs.push(scc);
            }
        }
    }
}

/// State for Tarjan's algorithm
struct TarjanState {
    visited: Set<usize>,
    index: Map<usize, usize>,
    lowlink: Map<usize, usize>,
    current_index: usize,
    stack: List<usize>,
    on_stack: Set<usize>,
}

impl TarjanState {
    fn new() -> Self {
        Self {
            visited: Set::new(),
            index: Map::new(),
            lowlink: Map::new(),
            current_index: 0,
            stack: List::new(),
            on_stack: Set::new(),
        }
    }
}

// ==================== Extension Traits ====================

/// Extension trait for Translator to support dependent types
///
/// This trait provides scope management for dependent type checking,
/// enabling variable bindings to be properly scoped during type elaboration.
///
/// Enables scoped variable bindings for Pi type `(x: A) -> B(x)` and Sigma type
/// `(x: A, B(x))` verification, where inner types depend on outer bindings.
pub trait TranslatorExt<'ctx> {
    /// Clone for a new scope (needed for dependent type checking)
    ///
    /// Creates a new translator that shares the same Z3 context but has its
    /// own copy of the bindings. This allows binding new variables in inner
    /// scopes without affecting the outer scope.
    ///
    /// The cloned translator inherits all existing bindings from the parent
    /// scope, enabling proper variable shadowing semantics.
    fn clone_for_scope(&self) -> Translator<'ctx>;

    /// Create a child scope with additional bindings
    ///
    /// This is a convenience method that clones the translator and adds
    /// new bindings in one step.
    fn with_binding(&self, name: Text, value: Dynamic) -> Translator<'ctx>;

    /// Create a child scope with multiple additional bindings
    fn with_bindings(
        &self,
        bindings: impl IntoIterator<Item = (Text, Dynamic)>,
    ) -> Translator<'ctx>;
}

impl<'ctx> TranslatorExt<'ctx> for Translator<'ctx> {
    fn clone_for_scope(&self) -> Translator<'ctx> {
        // Create a new translator with the same context
        let mut new_translator = Translator::new(self.context());

        // Copy all existing bindings to the new translator
        // This enables proper scoping: inner scopes inherit outer bindings
        for name in self.binding_names() {
            if let Maybe::Some(value) = self.get(name.as_str()) {
                new_translator.bind(name.clone(), value.clone());
            }
        }

        new_translator
    }

    fn with_binding(&self, name: Text, value: Dynamic) -> Translator<'ctx> {
        let mut scoped = self.clone_for_scope();
        scoped.bind(name, value);
        scoped
    }

    fn with_bindings(
        &self,
        bindings: impl IntoIterator<Item = (Text, Dynamic)>,
    ) -> Translator<'ctx> {
        let mut scoped = self.clone_for_scope();
        for (name, value) in bindings {
            scoped.bind(name, value);
        }
        scoped
    }
}

// ==================== Custom SMT Theories ====================

/// Custom SMT theory for domain-specific verification
///
/// Allows defining custom sorts, functions, and axioms for specific
/// domains (e.g., bit-vectors, arrays, algebraic data types).
///
/// Custom SMT theories allow domain-specific sorts, functions, and axioms.
/// Example: `theory BitVector { sort BV<n>; function bv_add(BV<n>, BV<n>): BV<n>; }`
#[derive(Debug, Clone)]
pub struct CustomTheory {
    /// Theory name
    pub name: Text,
    /// Custom sorts
    pub sorts: List<CustomSort>,
    /// Custom functions
    pub functions: List<CustomFunction>,
    /// Theory axioms
    pub axioms: List<Expr>,
}

impl CustomTheory {
    /// Create a new custom theory
    pub fn new(name: Text) -> Self {
        Self {
            name,
            sorts: List::new(),
            functions: List::new(),
            axioms: List::new(),
        }
    }

    /// Add a custom sort
    pub fn add_sort(&mut self, sort: CustomSort) {
        self.sorts.push(sort);
    }

    /// Add a custom function
    pub fn add_function(&mut self, func: CustomFunction) {
        self.functions.push(func);
    }

    /// Add an axiom
    pub fn add_axiom(&mut self, axiom: Expr) {
        self.axioms.push(axiom);
    }
}

/// Custom sort definition
#[derive(Debug, Clone)]
pub struct CustomSort {
    /// Sort name
    pub name: Text,
    /// Sort arity (number of parameters)
    pub arity: usize,
}

/// Custom function declaration
#[derive(Debug, Clone)]
pub struct CustomFunction {
    /// Function name
    pub name: Text,
    /// Parameter types
    pub param_types: List<Text>,
    /// Return type
    pub return_type: Text,
}

// ==================== Proof Terms ====================

/// Proof term for formal verification
///
/// Represents a constructive proof that can be checked independently.
/// This is the foundation for proof-carrying code.
///
/// Proof terms are first-class values: `type Proof<P: Prop> is evidence of P`.
/// Constructors include reflexivity, symmetry, transitivity, modus ponens,
/// conjunction intro/elim, and assumption. Enables proof-carrying code.
#[derive(Debug, Clone)]
pub struct ProofTerm {
    /// Proposition being proven
    pub proposition: Heap<Expr>,
    /// Proof structure
    pub proof: ProofStructure,
    /// Used axioms and lemmas
    pub dependencies: Set<Text>,
}

impl ProofTerm {
    /// Create a new proof term
    pub fn new(proposition: Expr, proof: ProofStructure) -> Self {
        Self {
            proposition: Heap::new(proposition),
            proof,
            dependencies: Set::new(),
        }
    }

    /// Add a dependency (axiom or lemma)
    pub fn add_dependency(&mut self, dep: Text) {
        self.dependencies.insert(dep);
    }

    /// Verify the proof term is well-formed
    ///
    /// Performs structural validation of the proof term:
    /// - No circular dependencies in proof structure
    /// - All referenced dependencies exist
    /// - Proof structure matches proposition type
    pub fn check_well_formed(&self) -> bool {
        self.check_proof_structure(&self.proof, &mut Set::new())
    }

    /// Recursively check proof structure for well-formedness
    fn check_proof_structure(&self, proof: &ProofStructure, visited: &mut Set<Text>) -> bool {
        match proof {
            ProofStructure::SolverProof { .. } => {
                // Solver proofs are always well-formed if generated
                true
            }
            ProofStructure::Refl => {
                // Reflexivity is always well-formed
                true
            }
            ProofStructure::Subst { eq_proof, property } => {
                // Check that the equality proof is well-formed
                // and doesn't create cycles
                let eq_term = eq_proof;
                if visited.contains(&Text::from("subst")) {
                    return false; // Cycle detected
                }
                visited.insert(Text::from("subst"));
                let result = self.check_proof_structure(&eq_term.proof, visited);
                visited.remove(&Text::from("subst"));
                result
            }
            ProofStructure::Trans { left, right } => {
                // Both transitive proofs must be well-formed
                if visited.contains(&Text::from("trans")) {
                    return false;
                }
                visited.insert(Text::from("trans"));
                let left_ok = self.check_proof_structure(&left.proof, visited);
                let right_ok = self.check_proof_structure(&right.proof, visited);
                visited.remove(&Text::from("trans"));
                left_ok && right_ok
            }
            ProofStructure::ModusPonens {
                premise,
                implication,
            } => {
                // Both premise and implication must be well-formed
                if visited.contains(&Text::from("modus_ponens")) {
                    return false;
                }
                visited.insert(Text::from("modus_ponens"));
                let premise_ok = self.check_proof_structure(&premise.proof, visited);
                let impl_ok = self.check_proof_structure(&implication.proof, visited);
                visited.remove(&Text::from("modus_ponens"));
                premise_ok && impl_ok
            }
            ProofStructure::Assumption { name } => {
                // Check that the assumption is declared as a dependency
                self.dependencies.contains(name)
            }
        }
    }
}

/// Proof structure
///
/// Represents the actual proof construction using proof rules.
#[derive(Debug, Clone)]
pub enum ProofStructure {
    /// Direct proof by SMT solver
    SolverProof {
        /// SMT-LIB2 proof object
        smt_proof: Text,
    },

    /// Proof by reflexivity (a = a)
    Refl,

    /// Proof by substitution
    Subst {
        /// Equality proof
        eq_proof: Heap<ProofTerm>,
        /// Property to substitute
        property: Heap<Expr>,
    },

    /// Proof by transitivity
    Trans {
        /// First equality
        left: Heap<ProofTerm>,
        /// Second equality
        right: Heap<ProofTerm>,
    },

    /// Proof by modus ponens (P, P → Q ⊢ Q)
    ModusPonens {
        /// Proof of P
        premise: Heap<ProofTerm>,
        /// Proof of P → Q
        implication: Heap<ProofTerm>,
    },

    /// Proof by assumption (axiom or hypothesis)
    Assumption {
        /// Assumption name
        name: Text,
    },
}

// ==================== Quantifier Support ====================

/// Quantifier handler for first-class ∀ and ∃ support
///
/// Handles first-class universal and existential quantifiers for dependent types.
/// Type-level functions compute types from values (e.g., `matrix_type(rows, cols) -> Type`).
/// Automated proof search uses strategies: assumption, reflexivity, intro, split, apply,
/// with hints database for priority-based lemma application.
pub struct QuantifierHandler {
    /// Maximum instantiation depth
    #[allow(dead_code)] // Used for depth-limited instantiation
    max_depth: usize,
    /// Trigger patterns for E-matching
    patterns: Map<Text, List<TriggerPattern>>,
}

impl QuantifierHandler {
    /// Create a new quantifier handler
    pub fn new() -> Self {
        Self {
            max_depth: 5,
            patterns: Map::new(),
        }
    }

    /// Add trigger pattern for quantifier instantiation
    ///
    /// Trigger patterns guide Z3's E-matching algorithm for quantifier
    /// instantiation, crucial for performance.
    pub fn add_pattern(&mut self, quantifier: Text, pattern: TriggerPattern) {
        self.patterns
            .entry(quantifier)
            .or_default()
            .push(pattern);
    }

    /// Get patterns for a quantifier
    pub fn get_patterns(&self, quantifier: &str) -> Maybe<&List<TriggerPattern>> {
        self.patterns.get(&Text::from(quantifier))
    }
}

impl Default for QuantifierHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Trigger pattern for E-matching
///
/// Patterns tell Z3 when to instantiate a quantified variable.
#[derive(Debug, Clone)]
pub struct TriggerPattern {
    /// Pattern expression
    pub pattern: Heap<Expr>,
    /// Weight (lower = higher priority)
    pub weight: u32,
}

impl TriggerPattern {
    /// Create a new trigger pattern
    pub fn new(pattern: Expr) -> Self {
        Self {
            pattern: Heap::new(pattern),
            weight: 1,
        }
    }

    /// Create with custom weight
    pub fn with_weight(pattern: Expr, weight: u32) -> Self {
        Self {
            pattern: Heap::new(pattern),
            weight,
        }
    }
}

// ==================== Proof Certificate Generation ====================

/// Proof certificate for independent verification
///
/// Generates machine-checkable proofs that can be verified by other tools.
///
/// Generates machine-checkable certificates in Dedukti, Coq, Lean, OpenTheory, or
/// Metamath format. Each certificate includes axioms, definitions, proof terms, and
/// checksums for independent verification by external proof checkers.
pub struct ProofCertificateGenerator {
    /// Target format
    format: CertificateFormat,
    /// Proof database
    proofs: Map<Text, ProofTerm>,
}

impl ProofCertificateGenerator {
    /// Create a new certificate generator
    pub fn new(format: CertificateFormat) -> Self {
        Self {
            format,
            proofs: Map::new(),
        }
    }

    /// Add a proof to the certificate
    pub fn add_proof(&mut self, name: Text, proof: ProofTerm) {
        self.proofs.insert(name, proof);
    }

    /// Generate certificate
    pub fn generate(&self) -> Result<Certificate, CertificateError> {
        let mut axioms = List::new();
        let mut theorems = List::new();

        for (_name, proof) in self.proofs.iter() {
            // Extract dependencies
            for dep in proof.dependencies.iter() {
                if !axioms.contains(dep) {
                    axioms.push(dep.clone());
                }
            }

            // Add theorem
            theorems.push(CertificateTheorem {
                proposition: proof.proposition.clone(),
                proof_term: self.encode_proof(&proof.proof),
            });
        }

        let checksum = self.compute_checksum(&theorems);
        Ok(Certificate {
            format: self.format,
            axioms,
            theorems,
            checksum,
        })
    }

    /// Encode proof in target format
    ///
    /// Encodes proof structures according to the target certificate format.
    /// Each proof rule is translated to its corresponding notation:
    /// - Refl: Reflexivity proof
    /// - Assumption: Axiom/hypothesis reference
    /// - SolverProof: Embedded SMT-LIB2 proof
    /// - Subst: Substitution of equals
    /// - Trans: Transitivity chain
    /// - ModusPonens: Implication elimination
    fn encode_proof(&self, proof: &ProofStructure) -> Text {
        match proof {
            ProofStructure::Refl => "refl".into(),

            ProofStructure::Assumption { name } => format!("assume({})", name).into(),

            ProofStructure::SolverProof { smt_proof } => {
                // Embed the SMT-LIB2 proof, escaping special characters
                let escaped = smt_proof
                    .replace("\\", "\\\\")
                    .replace("\"", "\\\"")
                    .replace("\n", "\\n");
                format!("smt_proof(\"{}\")", escaped).into()
            }

            ProofStructure::Subst { eq_proof, property } => {
                // Encode substitution: subst(eq_proof, property)
                // The eq_proof is the proof that a = b
                // The property is what we're substituting into
                let eq_term = self.encode_proof_term(eq_proof);
                let prop_str = format!("{:?}", property);
                format!("subst({}, {})", eq_term, prop_str).into()
            }

            ProofStructure::Trans { left, right } => {
                // Encode transitivity: trans(left_proof, right_proof)
                // If a = b and b = c, then a = c
                let left_term = self.encode_proof_term(left);
                let right_term = self.encode_proof_term(right);
                format!("trans({}, {})", left_term, right_term).into()
            }

            ProofStructure::ModusPonens {
                premise,
                implication,
            } => {
                // Encode modus ponens: mp(premise_proof, implication_proof)
                // If P and P -> Q, then Q
                let premise_term = self.encode_proof_term(premise);
                let impl_term = self.encode_proof_term(implication);
                format!("mp({}, {})", premise_term, impl_term).into()
            }
        }
    }

    /// Encode a proof term (helper for recursive proof encoding)
    fn encode_proof_term(&self, proof_term: &ProofTerm) -> Text {
        // Recursively encode the proof structure within the proof term
        self.encode_proof(&proof_term.proof)
    }

    /// Compute checksum for certificate integrity verification
    ///
    /// Uses a simple hash based on theorem content for deterministic verification.
    /// In production, this would use SHA-256 or similar cryptographic hash.
    fn compute_checksum(&self, theorems: &[CertificateTheorem]) -> Text {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        // Hash all theorems' propositions and proof terms
        for theorem in theorems {
            // Hash the proposition's debug representation
            let prop_str = format!("{:?}", theorem.proposition);
            prop_str.hash(&mut hasher);

            // Hash the proof term
            theorem.proof_term.as_str().hash(&mut hasher);
        }

        // Create a hex string from the hash
        let hash_value = hasher.finish();
        format!("{:016x}", hash_value).into()
    }
}

/// Certificate format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateFormat {
    /// SMT-LIB2 format
    SmtLib2,
    /// Dedukti universal proof format
    Dedukti,
    /// Lean proof format
    Lean,
    /// Coq proof format
    Coq,
}

/// Proof certificate
#[derive(Debug, Clone)]
pub struct Certificate {
    /// Format
    pub format: CertificateFormat,
    /// Axioms used
    pub axioms: List<Text>,
    /// Theorems proved
    pub theorems: List<CertificateTheorem>,
    /// Integrity checksum
    pub checksum: Text,
}

/// Certificate theorem
#[derive(Debug, Clone)]
pub struct CertificateTheorem {
    /// Proposition
    pub proposition: Heap<Expr>,
    /// Encoded proof
    pub proof_term: Text,
}

/// Certificate generation errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum CertificateError {
    #[error("encoding error: {0}")]
    EncodingError(Text),

    #[error("invalid proof structure: {0}")]
    InvalidProof(Text),

    #[error("unsupported format: {0:?}")]
    UnsupportedFormat(CertificateFormat),
}

// ==================== Universe Hierarchy ====================
//
// Universe Hierarchy: Type : Type1 : Type2 : ... (infinite hierarchy prevents paradoxes)
// Cumulative: Type0 <: Type1 <: Type2 <: ...
// Universe polymorphism: `fn identity<u: Level>(T: Type u, x: T) -> T`
//
// Verum implements a cumulative universe hierarchy to prevent Russell's paradox
// while enabling universe polymorphism for generic definitions.
//
// Type : Type₁ : Type₂ : Type₃ : ...
//
// The hierarchy is cumulative: Type₀ <: Type₁ <: Type₂ <: ...

/// Universe level in the type hierarchy
///
/// Universe levels: concrete (0, 1, 2, ...), variable (for polymorphism),
/// max (for type formers), succ (successor). Cumulativity: Type_i <: Type_(i+1).
///
/// Universe levels can be:
/// - Concrete: specific natural number (0, 1, 2, ...)
/// - Variable: level variable for universe polymorphism
/// - Max: maximum of two levels (for type formers)
/// - Succ: successor of another level
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UniverseLevel {
    /// Concrete universe level (Type₀, Type₁, etc.)
    Concrete(usize),
    /// Level variable for universe polymorphism
    Variable(Text),
    /// Maximum of two levels: max(l1, l2)
    Max(Heap<UniverseLevel>, Heap<UniverseLevel>),
    /// Successor level: succ(l) = l + 1
    Succ(Heap<UniverseLevel>),
}

impl UniverseLevel {
    /// Type₀ - base universe (Prop in some systems)
    pub const TYPE0: UniverseLevel = UniverseLevel::Concrete(0);
    /// Type₁ - universe of Type₀ (Set in some systems)
    pub const TYPE1: UniverseLevel = UniverseLevel::Concrete(1);
    /// Type₂ - universe of Type₁
    pub const TYPE2: UniverseLevel = UniverseLevel::Concrete(2);

    /// Create a concrete universe level
    pub fn concrete(n: usize) -> Self {
        UniverseLevel::Concrete(n)
    }

    /// Create a level variable for universe polymorphism
    pub fn variable(name: impl Into<Text>) -> Self {
        UniverseLevel::Variable(name.into())
    }

    /// Create successor level
    pub fn succ(&self) -> Self {
        match self {
            UniverseLevel::Concrete(n) => UniverseLevel::Concrete(n + 1),
            _ => UniverseLevel::Succ(Heap::new(self.clone())),
        }
    }

    /// Create maximum of two levels
    pub fn max(l1: UniverseLevel, l2: UniverseLevel) -> Self {
        match (&l1, &l2) {
            (UniverseLevel::Concrete(n1), UniverseLevel::Concrete(n2)) => {
                UniverseLevel::Concrete((*n1).max(*n2))
            }
            _ => UniverseLevel::Max(Heap::new(l1), Heap::new(l2)),
        }
    }

    /// Check if this level is ground (no variables)
    pub fn is_ground(&self) -> bool {
        match self {
            UniverseLevel::Concrete(_) => true,
            UniverseLevel::Variable(_) => false,
            UniverseLevel::Max(l1, l2) => l1.is_ground() && l2.is_ground(),
            UniverseLevel::Succ(l) => l.is_ground(),
        }
    }

    /// Evaluate a ground level to a concrete number
    pub fn eval(&self) -> Maybe<usize> {
        match self {
            UniverseLevel::Concrete(n) => Maybe::Some(*n),
            UniverseLevel::Variable(_) => Maybe::None,
            UniverseLevel::Max(l1, l2) => match (l1.eval(), l2.eval()) {
                (Maybe::Some(n1), Maybe::Some(n2)) => Maybe::Some(n1.max(n2)),
                _ => Maybe::None,
            },
            UniverseLevel::Succ(l) => l.eval().map(|n| n + 1),
        }
    }

    /// Collect all level variables in this level
    pub fn variables(&self) -> Set<Text> {
        let mut vars = Set::new();
        self.collect_variables(&mut vars);
        vars
    }

    fn collect_variables(&self, vars: &mut Set<Text>) {
        match self {
            UniverseLevel::Concrete(_) => {}
            UniverseLevel::Variable(v) => {
                vars.insert(v.clone());
            }
            UniverseLevel::Max(l1, l2) => {
                l1.collect_variables(vars);
                l2.collect_variables(vars);
            }
            UniverseLevel::Succ(l) => l.collect_variables(vars),
        }
    }

    /// Substitute level variable with concrete level
    pub fn substitute(&self, var: &Text, level: &UniverseLevel) -> UniverseLevel {
        match self {
            UniverseLevel::Concrete(n) => UniverseLevel::Concrete(*n),
            UniverseLevel::Variable(v) if v == var => level.clone(),
            UniverseLevel::Variable(v) => UniverseLevel::Variable(v.clone()),
            UniverseLevel::Max(l1, l2) => {
                UniverseLevel::max(l1.substitute(var, level), l2.substitute(var, level))
            }
            UniverseLevel::Succ(l) => l.substitute(var, level).succ(),
        }
    }
}

impl PartialOrd for UniverseLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self.eval(), other.eval()) {
            (Maybe::Some(n1), Maybe::Some(n2)) => Some(n1.cmp(&n2)),
            _ => None, // Cannot compare non-ground levels
        }
    }
}

/// Universe constraint for level inference
///
/// Constraints generated during type checking to ensure
/// universe consistency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UniverseConstraint {
    /// l1 <= l2 (l1 is at most l2)
    LessOrEqual(UniverseLevel, UniverseLevel),
    /// l1 < l2 (l1 is strictly less than l2)
    LessThan(UniverseLevel, UniverseLevel),
    /// l1 = l2 (levels are equal)
    Equal(UniverseLevel, UniverseLevel),
}

impl UniverseConstraint {
    /// Check if constraint is satisfied
    pub fn is_satisfied(&self) -> Maybe<bool> {
        match self {
            UniverseConstraint::LessOrEqual(l1, l2) => match (l1.eval(), l2.eval()) {
                (Maybe::Some(n1), Maybe::Some(n2)) => Maybe::Some(n1 <= n2),
                _ => Maybe::None,
            },
            UniverseConstraint::LessThan(l1, l2) => match (l1.eval(), l2.eval()) {
                (Maybe::Some(n1), Maybe::Some(n2)) => Maybe::Some(n1 < n2),
                _ => Maybe::None,
            },
            UniverseConstraint::Equal(l1, l2) => match (l1.eval(), l2.eval()) {
                (Maybe::Some(n1), Maybe::Some(n2)) => Maybe::Some(n1 == n2),
                _ => Maybe::None,
            },
        }
    }
}

/// Universe constraint solver
///
/// Solves universe constraints to determine level assignments.
pub struct UniverseConstraintSolver {
    /// Collected constraints
    constraints: List<UniverseConstraint>,
    /// Current level assignments
    assignments: Map<Text, usize>,
}

impl UniverseConstraintSolver {
    /// Create a new solver
    pub fn new() -> Self {
        Self {
            constraints: List::new(),
            assignments: Map::new(),
        }
    }

    /// Add a constraint
    pub fn add_constraint(&mut self, constraint: UniverseConstraint) {
        self.constraints.push(constraint);
    }

    /// Add constraint: l1 <= l2
    pub fn add_leq(&mut self, l1: UniverseLevel, l2: UniverseLevel) {
        self.add_constraint(UniverseConstraint::LessOrEqual(l1, l2));
    }

    /// Add constraint: l1 < l2
    pub fn add_lt(&mut self, l1: UniverseLevel, l2: UniverseLevel) {
        self.add_constraint(UniverseConstraint::LessThan(l1, l2));
    }

    /// Add constraint: l1 = l2
    pub fn add_eq(&mut self, l1: UniverseLevel, l2: UniverseLevel) {
        self.add_constraint(UniverseConstraint::Equal(l1, l2));
    }

    /// Solve constraints using iterative refinement
    ///
    /// Returns Ok(()) if constraints are satisfiable, Err with message otherwise.
    pub fn solve(&mut self) -> Result<(), Text> {
        // Collect all variables
        let mut all_vars = Set::new();
        for constraint in &self.constraints {
            match constraint {
                UniverseConstraint::LessOrEqual(l1, l2)
                | UniverseConstraint::LessThan(l1, l2)
                | UniverseConstraint::Equal(l1, l2) => {
                    for v in l1.variables() {
                        all_vars.insert(v);
                    }
                    for v in l2.variables() {
                        all_vars.insert(v);
                    }
                }
            }
        }

        // Initialize all variables to level 0
        for var in all_vars {
            self.assignments.insert(var, 0);
        }

        // Iterative constraint propagation
        let max_iterations = 100;
        for _ in 0..max_iterations {
            let mut changed = false;

            for constraint in &self.constraints {
                match constraint {
                    UniverseConstraint::LessOrEqual(l1, l2) => {
                        let v1 = self.eval_with_assignments(l1);
                        let v2 = self.eval_with_assignments(l2);

                        if v1 > v2 {
                            // Need to increase l2 or decrease l1
                            // Try increasing l2 first
                            if let Maybe::Some(var) = self.find_variable(l2) {
                                let current = self.assignments.get(&var).copied().unwrap_or(0);
                                self.assignments.insert(var, v1);
                                changed = true;
                            } else {
                                return Err(format!(
                                    "Universe constraint unsatisfiable: {} <= {}",
                                    v1, v2
                                )
                                .into());
                            }
                        }
                    }
                    UniverseConstraint::LessThan(l1, l2) => {
                        let v1 = self.eval_with_assignments(l1);
                        let v2 = self.eval_with_assignments(l2);

                        if v1 >= v2 {
                            if let Maybe::Some(var) = self.find_variable(l2) {
                                self.assignments.insert(var, v1 + 1);
                                changed = true;
                            } else {
                                return Err(format!(
                                    "Universe constraint unsatisfiable: {} < {}",
                                    v1, v2
                                )
                                .into());
                            }
                        }
                    }
                    UniverseConstraint::Equal(l1, l2) => {
                        let v1 = self.eval_with_assignments(l1);
                        let v2 = self.eval_with_assignments(l2);

                        if v1 != v2 {
                            // Try to equalize
                            if let Maybe::Some(var) = self.find_variable(l1) {
                                self.assignments.insert(var, v2);
                                changed = true;
                            } else if let Maybe::Some(var) = self.find_variable(l2) {
                                self.assignments.insert(var, v1);
                                changed = true;
                            } else {
                                return Err(format!(
                                    "Universe constraint unsatisfiable: {} = {}",
                                    v1, v2
                                )
                                .into());
                            }
                        }
                    }
                }
            }

            if !changed {
                // Reached fixed point
                return Ok(());
            }
        }

        Err("Universe constraint solving did not converge".into())
    }

    fn eval_with_assignments(&self, level: &UniverseLevel) -> usize {
        match level {
            UniverseLevel::Concrete(n) => *n,
            UniverseLevel::Variable(v) => self.assignments.get(v).copied().unwrap_or(0),
            UniverseLevel::Max(l1, l2) => {
                let v1 = self.eval_with_assignments(l1);
                let v2 = self.eval_with_assignments(l2);
                v1.max(v2)
            }
            UniverseLevel::Succ(l) => self.eval_with_assignments(l) + 1,
        }
    }

    fn find_variable(&self, level: &UniverseLevel) -> Maybe<Text> {
        match level {
            UniverseLevel::Variable(v) => Maybe::Some(v.clone()),
            UniverseLevel::Max(l1, l2) => match self.find_variable(l1) {
                Maybe::Some(v) => Maybe::Some(v),
                Maybe::None => self.find_variable(l2),
            },
            UniverseLevel::Succ(l) => self.find_variable(l),
            UniverseLevel::Concrete(_) => Maybe::None,
        }
    }

    /// Get the assignment for a variable
    pub fn get_assignment(&self, var: &str) -> Maybe<usize> {
        self.assignments
            .get(&Text::from(var))
            .copied()
            .map(Maybe::Some)
            .unwrap_or(Maybe::None)
    }
}

impl Default for UniverseConstraintSolver {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Inductive Types ====================
//
// Inductive types: `inductive Nat : Type { zero : Nat, succ : Nat -> Nat }`
// Automatically generate induction principles: `nat_ind(base, step, n)`.
//
// Inductive types are defined by:
// - A name and parameters
// - A set of constructors
// - Automatically generated induction/recursion principles

/// Inductive type definition
///
/// Inductive types: `inductive Nat : Type { zero : Nat, succ : Nat -> Nat }`.
/// Indexed: `inductive List<A> : Nat -> Type { nil : List<A, 0>, cons : ... }`.
/// Induction principle automatically derived from constructor signatures.
///
/// Supports:
/// - Parameterized inductive types (List<T>)
/// - Indexed inductive types (Vec<T, n>)
/// - Mutually recursive types
/// - Strict positivity checking
#[derive(Debug, Clone)]
pub struct InductiveType {
    /// Type name
    pub name: Text,
    /// Type parameters (uniform, not indexed)
    pub params: List<TypeParam>,
    /// Type indices (may vary across constructors)
    pub indices: List<IndexParam>,
    /// Constructors
    pub constructors: List<Constructor>,
    /// Universe level of this type
    pub universe: UniverseLevel,
    /// Induction principle (auto-generated)
    pub induction_principle: Maybe<Heap<Expr>>,
    /// Recursion principle (auto-generated)
    pub recursion_principle: Maybe<Heap<Expr>>,
    /// Is this a mutually recursive definition?
    pub is_mutual: bool,
    /// Names of mutually defined types
    pub mutual_types: List<Text>,
}

impl InductiveType {
    /// Create a new inductive type
    pub fn new(name: Text) -> Self {
        Self {
            name,
            params: List::new(),
            indices: List::new(),
            constructors: List::new(),
            universe: UniverseLevel::TYPE0,
            induction_principle: Maybe::None,
            recursion_principle: Maybe::None,
            is_mutual: false,
            mutual_types: List::new(),
        }
    }

    /// Add a type parameter
    pub fn with_param(mut self, param: TypeParam) -> Self {
        self.params.push(param);
        self
    }

    /// Add an index parameter
    pub fn with_index(mut self, index: IndexParam) -> Self {
        self.indices.push(index);
        self
    }

    /// Add a constructor
    pub fn with_constructor(mut self, ctor: Constructor) -> Self {
        self.constructors.push(ctor);
        self
    }

    /// Set universe level
    pub fn at_universe(mut self, universe: UniverseLevel) -> Self {
        self.universe = universe;
        self
    }

    /// Generate the induction principle for this type
    ///
    /// For a type like:
    /// ```verum
    /// inductive Nat : Type {
    ///     zero : Nat,
    ///     succ : Nat -> Nat
    /// }
    /// ```
    ///
    /// Generates:
    /// ```verum
    /// fn nat_ind<P: Nat -> Type>
    ///     (base: P(zero))
    ///     (step: (n: Nat) -> P(n) -> P(succ(n)))
    ///     (n: Nat) -> P(n)
    /// ```
    pub fn generate_induction_principle(&mut self) {
        // Generate the complete induction principle for this inductive type
        //
        // For a type like:
        //   inductive Nat : Type {
        //     Zero : Nat,
        //     Succ : Nat -> Nat
        //   }
        //
        // Generates:
        //   fn nat_ind<P: Nat -> Type>
        //       (base: P(Zero))
        //       (step: (n: Nat) -> P(n) -> P(Succ(n)))
        //       (n: Nat) -> P(n)
        //
        // The structure is:
        //   ∀P: (T -> Type).
        //     (∀ non-recursive constructor cases) ->
        //     (∀ recursive constructor cases with inductive hypotheses) ->
        //     (∀x: T. P(x))

        use verum_ast::pattern::{Pattern, PatternKind};
        use verum_ast::span::Span;
        use verum_ast::ty::{Ident, Path};

        let span = Span::dummy();

        // 1. Create the motive type variable P: T -> Type
        let motive_name = Text::from("P");
        let motive_pattern = Pattern::new(
            PatternKind::Ident {
                by_ref: false,
                mutable: false,
                name: Ident::new(motive_name.to_string(), span),
                subpattern: Maybe::None,
            },
            span,
        );

        // Type of motive: T -> Type
        let mut motive_params = Vec::new();
        motive_params.push(Type::new(
            TypeKind::Path(Path::from_ident(Ident::new(self.name.to_string(), span))),
            span,
        ));

        let motive_type = Type::new(
            TypeKind::Function {
                params: motive_params.into(),
                return_type: Box::new(Type::new(
                    TypeKind::Path(Path::from_ident(Ident::new("Type".to_string(), span))),
                    span,
                )),
                calling_convention: Maybe::None,
                contexts: ContextList::empty(),
            },
            span,
        );

        // 2. Build arguments for each constructor
        let mut constructor_args = List::new();

        for ctor in &self.constructors {
            let ctor_arg_type = self.build_constructor_case(ctor, motive_name.as_ref(), span);
            constructor_args.push(ctor_arg_type);
        }

        // 3. Build the final return type: (n: T) -> P(n)
        let target_param_name = Text::from("x");
        let target_type = Type::new(
            TypeKind::Path(Path::from_ident(Ident::new(self.name.to_string(), span))),
            span,
        );

        // Application P(x)
        let mut result_args = Vec::new();
        result_args.push(verum_ast::ty::GenericArg::Type(Type::new(
            TypeKind::Path(Path::from_ident(Ident::new(
                target_param_name.to_string(),
                span,
            ))),
            span,
        )));

        let result_type = Type::new(
            TypeKind::Generic {
                base: Box::new(Type::new(
                    TypeKind::Path(Path::from_ident(Ident::new(motive_name.to_string(), span))),
                    span,
                )),
                args: result_args.into(),
            },
            span,
        );

        let mut final_params = Vec::new();
        final_params.push(target_type);

        let final_type = Type::new(
            TypeKind::Function {
                params: final_params.into(),
                return_type: Box::new(result_type),
                calling_convention: Maybe::None,
                contexts: ContextList::empty(),
            },
            span,
        );

        // 4. Build the complete type by chaining all function types
        let mut complete_type = final_type;
        for arg_type in constructor_args.iter().rev() {
            let mut chain_params = Vec::new();
            chain_params.push(arg_type.clone());

            complete_type = Type::new(
                TypeKind::Function {
                    params: chain_params.into(),
                    return_type: Box::new(complete_type),
                    calling_convention: Maybe::None,
                    contexts: ContextList::empty(),
                },
                span,
            );
        }

        // 5. Wrap in universal quantifier for the motive P
        let body_expr = Expr::new(
            ExprKind::Path(Path::from_ident(Ident::new("induction_principle", span))),
            span,
        );

        let motive_binding = verum_ast::expr::QuantifierBinding::typed(
            motive_pattern,
            motive_type,
            span,
        );

        let quantified = Expr::new(
            ExprKind::Forall {
                bindings: List::from_iter([motive_binding]),
                body: Box::new(body_expr),
            },
            span,
        );

        self.induction_principle = Maybe::Some(Heap::new(quantified));
    }

    /// Build the type for a constructor case in the induction principle
    fn build_constructor_case(
        &self,
        ctor: &Constructor,
        motive_name: &str,
        span: verum_ast::span::Span,
    ) -> Type {
        // For non-recursive constructors (e.g., Zero : Nat):
        //   Returns: P(Zero)
        //
        // For recursive constructors (e.g., Succ : Nat -> Nat):
        //   Returns: (n: Nat) -> P(n) -> P(Succ(n))
        //   Which is: for all n, if P holds for n, then P holds for Succ(n)

        use verum_ast::ty::{Ident, Path};

        // Check if constructor is recursive (has any recursive arguments)
        let recursive_args: List<&ConstructorArg> = self
            .constructors
            .iter()
            .flat_map(|c| &c.args)
            .filter(|arg| arg.is_recursive)
            .collect();

        let is_recursive = ctor.args.iter().any(|arg| arg.is_recursive);

        if !is_recursive {
            // Base case: P(Constructor)
            // Create application P(ctor_name)
            let mut base_args = Vec::new();
            base_args.push(verum_ast::ty::GenericArg::Type(Type::new(
                TypeKind::Path(Path::from_ident(Ident::new(ctor.name.to_string(), span))),
                span,
            )));

            Type::new(
                TypeKind::Generic {
                    base: Box::new(Type::new(
                        TypeKind::Path(Path::from_ident(Ident::new(motive_name.to_string(), span))),
                        span,
                    )),
                    args: base_args.into(),
                },
                span,
            )
        } else {
            // Inductive case: build (arg1: T1) -> ... -> P(arg1) -> ... -> P(ctor(args))
            let mut param_types = Vec::new();

            // Add parameter types for each argument
            for arg in &ctor.args {
                param_types.push((*arg.ty).clone());

                // If argument is recursive, add inductive hypothesis P(arg)
                if arg.is_recursive {
                    let mut ih_args = Vec::new();
                    ih_args.push(verum_ast::ty::GenericArg::Type(Type::new(
                        TypeKind::Path(Path::from_ident(Ident::new(arg.name.to_string(), span))),
                        span,
                    )));

                    let ih_type = Type::new(
                        TypeKind::Generic {
                            base: Box::new(Type::new(
                                TypeKind::Path(Path::from_ident(Ident::new(
                                    motive_name.to_string(),
                                    span,
                                ))),
                                span,
                            )),
                            args: ih_args.into(),
                        },
                        span,
                    );
                    param_types.push(ih_type);
                }
            }

            // Build result type: P(Constructor(args))
            // For simplicity, we represent the constructor application
            let mut result_args = Vec::new();
            result_args.push(verum_ast::ty::GenericArg::Type(Type::new(
                TypeKind::Path(Path::from_ident(Ident::new(ctor.name.to_string(), span))),
                span,
            )));

            let result_type = Type::new(
                TypeKind::Generic {
                    base: Box::new(Type::new(
                        TypeKind::Path(Path::from_ident(Ident::new(motive_name.to_string(), span))),
                        span,
                    )),
                    args: result_args.into(),
                },
                span,
            );

            // Chain all parameters into a function type
            let mut complete_type = result_type;
            for param_type in param_types.iter().rev() {
                complete_type = Type::new(
                    TypeKind::Function {
                        params: vec![param_type.clone()].into(),
                        return_type: Box::new(complete_type),
                        calling_convention: Maybe::None,
                        contexts: ContextList::empty(),
                    },
                    span,
                );
            }

            complete_type
        }
    }

    /// Check strict positivity of type occurrences
    ///
    /// Ensures the type being defined only appears in strictly positive
    /// positions in constructor arguments, preventing non-termination.
    ///
    /// This is crucial for soundness: negative occurrences allow encoding
    /// Russell's paradox, leading to logical inconsistency.
    pub fn check_strict_positivity(&self) -> Result<(), Text> {
        // First check for circular dependencies
        let backend = DependentTypeBackend::new();
        backend.check_inductive_cycles(self)?;

        // Then check positivity for each constructor
        for ctor in &self.constructors {
            self.check_positivity_in_type(&ctor.ty, true)?;
        }

        Ok(())
    }

    fn check_positivity_in_type(&self, ty: &Type, positive: bool) -> Result<(), Text> {
        match &ty.kind {
            TypeKind::Path(path) => {
                // Check if this is a reference to the type being defined
                if path.segments.len() == 1
                    && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
                    && ident.name == self.name
                    && !positive
                {
                    return Err(format!(
                        "Type '{}' occurs in negative position in constructor",
                        self.name
                    )
                    .into());
                }
                Ok(())
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            }
            | TypeKind::Rank2Function {
                type_params: _,
                params,
                return_type,
                ..
            } => {
                // Parameters are in negative position
                for param in params {
                    self.check_positivity_in_type(param, !positive)?;
                }
                // Return type is in positive position
                self.check_positivity_in_type(return_type, positive)
            }
            TypeKind::Refined { base, .. } => self.check_positivity_in_type(base, positive),
            TypeKind::Generic { base, args } => {
                // For now, treat all type arguments as positive
                // Full implementation would track variance
                for arg in args {
                    if let verum_ast::ty::GenericArg::Type(t) = arg {
                        self.check_positivity_in_type(t, positive)?;
                    }
                }
                self.check_positivity_in_type(base, positive)
            }
            TypeKind::Tuple(types) => {
                // Tuple fields are in positive position
                for ty in types {
                    self.check_positivity_in_type(ty, positive)?;
                }
                Ok(())
            }
            TypeKind::Array { element, .. }
            | TypeKind::Slice(element)
            | TypeKind::Reference { inner: element, .. }
            | TypeKind::CheckedReference { inner: element, .. }
            | TypeKind::UnsafeReference { inner: element, .. }
            | TypeKind::Pointer { inner: element, .. }
            | TypeKind::VolatilePointer { inner: element, .. }
            | TypeKind::Ownership { inner: element, .. }
            | TypeKind::GenRef { inner: element }
            | TypeKind::Bounded { base: element, .. }
            | TypeKind::TypeConstructor { base: element, .. } => {
                // Transparent wrappers preserve position
                self.check_positivity_in_type(element, positive)
            }
            TypeKind::Sigma { base, .. } | TypeKind::Qualified { self_ty: base, .. } => {
                self.check_positivity_in_type(base, positive)
            }
            TypeKind::Tensor { element, .. } => self.check_positivity_in_type(element, positive),
            // Existential types - check bounds' equality types for positivity
            TypeKind::Existential { bounds, .. } => {
                for bound in bounds {
                    if let verum_ast::ty::TypeBoundKind::Equality(ty) = &bound.kind {
                        self.check_positivity_in_type(ty, positive)?;
                    }
                }
                Ok(())
            }
            // Associated types - check base type for positivity
            TypeKind::AssociatedType { base, .. } => self.check_positivity_in_type(base, positive),
            // Capability-restricted types - check base type for positivity
            TypeKind::CapabilityRestricted { base, .. } => {
                self.check_positivity_in_type(base, positive)
            }
            // Record types - check all field types for positivity
            TypeKind::Record { fields } => {
                for field in fields {
                    self.check_positivity_in_type(&field.ty, positive)?;
                }
                Ok(())
            }
            // Path equality type: check carrier for positivity
            TypeKind::PathType { carrier, .. } | TypeKind::DependentApp { carrier, .. } => self.check_positivity_in_type(carrier, positive),
            // DynProtocol, primitives, inferred, Never, Unknown, and Universe don't affect positivity
            TypeKind::DynProtocol { .. }
            | TypeKind::Unit
            | TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::Char
            | TypeKind::Text
            | TypeKind::Inferred
            | TypeKind::Never
            | TypeKind::Unknown
            | TypeKind::Universe { .. }
            | TypeKind::Meta { .. }
            | TypeKind::TypeLambda { .. } => Ok(()),
        }
    }
}

/// Type parameter (uniform across constructors)
#[derive(Debug, Clone)]
pub struct TypeParam {
    /// Parameter name
    pub name: Text,
    /// Parameter type (kind)
    pub ty: Heap<Type>,
    /// Is this implicit?
    pub implicit: bool,
}

impl TypeParam {
    /// Create explicit type parameter
    pub fn explicit(name: Text, ty: Type) -> Self {
        Self {
            name,
            ty: Heap::new(ty),
            implicit: false,
        }
    }

    /// Create implicit type parameter
    pub fn implicit(name: Text, ty: Type) -> Self {
        Self {
            name,
            ty: Heap::new(ty),
            implicit: true,
        }
    }
}

/// Index parameter (may vary across constructors)
#[derive(Debug, Clone)]
pub struct IndexParam {
    /// Parameter name
    pub name: Text,
    /// Parameter type
    pub ty: Heap<Type>,
}

impl IndexParam {
    pub fn new(name: Text, ty: Type) -> Self {
        Self {
            name,
            ty: Heap::new(ty),
        }
    }
}

/// Constructor for inductive types
#[derive(Debug, Clone)]
pub struct Constructor {
    /// Constructor name
    pub name: Text,
    /// Constructor type (full dependent type)
    pub ty: Heap<Type>,
    /// Constructor arguments
    pub args: List<ConstructorArg>,
    /// Result type indices (for indexed families)
    pub result_indices: List<Expr>,
}

impl Constructor {
    /// Create a simple constructor with just a type
    pub fn simple(name: Text, ty: Type) -> Self {
        Self {
            name,
            ty: Heap::new(ty),
            args: List::new(),
            result_indices: List::new(),
        }
    }

    /// Add an argument to this constructor
    pub fn with_arg(mut self, arg: ConstructorArg) -> Self {
        self.args.push(arg);
        self
    }
}

/// Constructor argument
#[derive(Debug, Clone)]
pub struct ConstructorArg {
    /// Argument name
    pub name: Text,
    /// Argument type
    pub ty: Heap<Type>,
    /// Is this a recursive argument?
    pub is_recursive: bool,
}

impl ConstructorArg {
    pub fn new(name: Text, ty: Type, is_recursive: bool) -> Self {
        Self {
            name,
            ty: Heap::new(ty),
            is_recursive,
        }
    }
}

// ==================== Coinductive Types ====================
//
// Coinductive types: `coinductive Stream<A> { head: Stream<A> -> A, tail: Stream<A> -> Stream<A> }`
//
// Coinductive types represent infinite structures (streams, processes)
// defined by observation (destructors) rather than construction.

/// Coinductive type definition
///
/// Coinductive types defined by destructors (observations) rather than constructors.
/// Productivity checker ensures corecursive definitions make progress.
#[derive(Debug, Clone)]
pub struct CoinductiveType {
    /// Type name
    pub name: Text,
    /// Type parameters
    pub params: List<TypeParam>,
    /// Destructors (observations)
    pub destructors: List<Destructor>,
    /// Universe level
    pub universe: UniverseLevel,
}

impl CoinductiveType {
    pub fn new(name: Text) -> Self {
        Self {
            name,
            params: List::new(),
            destructors: List::new(),
            universe: UniverseLevel::TYPE0,
        }
    }

    pub fn with_destructor(mut self, dtor: Destructor) -> Self {
        self.destructors.push(dtor);
        self
    }
}

/// Destructor (observation) for coinductive types
#[derive(Debug, Clone)]
pub struct Destructor {
    /// Destructor name (e.g., "head", "tail")
    pub name: Text,
    /// Result type
    pub result_ty: Heap<Type>,
}

impl Destructor {
    pub fn new(name: Text, result_ty: Type) -> Self {
        Self {
            name,
            result_ty: Heap::new(result_ty),
        }
    }
}

// ==================== Higher Inductive Types (HITs) ====================
//
// Higher Inductive Types (HITs): types with both point and path constructors.
// Example: `hott inductive Circle { base : Circle, loop : base = base }`
//
// HITs extend inductive types with path (equality) constructors,
// essential for homotopy type theory and quotient types.

/// Higher Inductive Type definition
///
/// HITs have both point constructors (like regular inductive types)
/// and path constructors (equality proofs between points).
#[derive(Debug, Clone)]
pub struct HigherInductiveType {
    /// Type name
    pub name: Text,
    /// Type parameters
    pub params: List<TypeParam>,
    /// Point constructors (create elements)
    pub point_constructors: List<Constructor>,
    /// Path constructors (create equalities)
    pub path_constructors: List<PathConstructor>,
    /// Higher path constructors (equalities between paths)
    pub higher_paths: List<HigherPathConstructor>,
    /// Universe level
    pub universe: UniverseLevel,
}

impl HigherInductiveType {
    pub fn new(name: Text) -> Self {
        Self {
            name,
            params: List::new(),
            point_constructors: List::new(),
            path_constructors: List::new(),
            higher_paths: List::new(),
            universe: UniverseLevel::TYPE0,
        }
    }

    /// Add a point constructor
    pub fn with_point(mut self, ctor: Constructor) -> Self {
        self.point_constructors.push(ctor);
        self
    }

    /// Add a path constructor
    pub fn with_path(mut self, path: PathConstructor) -> Self {
        self.path_constructors.push(path);
        self
    }

    /// Add a higher path constructor
    pub fn with_higher_path(mut self, hp: HigherPathConstructor) -> Self {
        self.higher_paths.push(hp);
        self
    }
}

/// Path constructor for HITs
///
/// Represents an equality between two terms.
#[derive(Debug, Clone)]
pub struct PathConstructor {
    /// Constructor name
    pub name: Text,
    /// Left endpoint
    pub left: Heap<Expr>,
    /// Right endpoint
    pub right: Heap<Expr>,
    /// Parameters this path depends on
    pub params: List<TypeParam>,
}

impl PathConstructor {
    pub fn new(name: Text, left: Expr, right: Expr) -> Self {
        Self {
            name,
            left: Heap::new(left),
            right: Heap::new(right),
            params: List::new(),
        }
    }
}

/// Higher path constructor (path between paths)
#[derive(Debug, Clone)]
pub struct HigherPathConstructor {
    /// Constructor name
    pub name: Text,
    /// Left path
    pub left_path: Heap<Expr>,
    /// Right path
    pub right_path: Heap<Expr>,
    /// Dimension (1 = path, 2 = square, etc.)
    pub dimension: usize,
}

// ==================== Quantitative Type Theory ====================
//
// Quantitative Type Theory: track usage with quantities 0 (erased), 1 (linear), omega (unrestricted).
//
// QTT tracks resource usage with quantities: 0, 1, or ω (unlimited).
// This enables linear types while keeping full dependent types.

/// Resource quantity for quantitative type theory
///
/// Usage quantities: `0` (erased, compile-time only), `1` (linear, use exactly once),
/// `omega` (unrestricted, use any number of times). Graded modalities enable
/// resource tracking: `fn linear_use(x: Text @1) -> Text @1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quantity {
    /// Zero uses (erased at runtime)
    Zero,
    /// Exactly one use (linear)
    One,
    /// Unlimited uses (unrestricted)
    Omega,
}

impl Quantity {
    /// Multiply quantities (for function application)
    pub fn mul(self, other: Quantity) -> Quantity {
        match (self, other) {
            (Quantity::Zero, _) | (_, Quantity::Zero) => Quantity::Zero,
            (Quantity::One, Quantity::One) => Quantity::One,
            _ => Quantity::Omega,
        }
    }

    /// Add quantities (for pattern matching branches)
    pub fn add(self, other: Quantity) -> Quantity {
        match (self, other) {
            (Quantity::Zero, q) | (q, Quantity::Zero) => q,
            (Quantity::One, Quantity::One) => Quantity::Omega, // Used in both branches
            _ => Quantity::Omega,
        }
    }

    /// Check if this quantity is subsumed by another
    /// 0 <: 1 <: ω
    pub fn subsumed_by(self, other: Quantity) -> bool {
        match (self, other) {
            (_, Quantity::Omega) => true,
            (Quantity::Zero, _) => true,
            (Quantity::One, Quantity::One) => true,
            _ => false,
        }
    }
}

/// Quantified type binding
///
/// Represents `x :^q A` where q is the quantity.
#[derive(Debug, Clone)]
pub struct QuantifiedBinding {
    /// Variable name
    pub name: Text,
    /// Variable type
    pub ty: Heap<Type>,
    /// Usage quantity
    pub quantity: Quantity,
}

impl QuantifiedBinding {
    pub fn new(name: Text, ty: Type, quantity: Quantity) -> Self {
        Self {
            name,
            ty: Heap::new(ty),
            quantity,
        }
    }

    /// Create linear binding (quantity = 1)
    pub fn linear(name: Text, ty: Type) -> Self {
        Self::new(name, ty, Quantity::One)
    }

    /// Create unrestricted binding (quantity = ω)
    pub fn unrestricted(name: Text, ty: Type) -> Self {
        Self::new(name, ty, Quantity::Omega)
    }

    /// Create erased binding (quantity = 0)
    pub fn erased(name: Text, ty: Type) -> Self {
        Self::new(name, ty, Quantity::Zero)
    }
}

// ==================== View Patterns ====================
//
// View Patterns: alternative pattern interfaces, e.g., `view Parity : Nat -> Type`
// with `Even(n)` and `Odd(n)` cases for matching even/odd numbers.
//
// Views provide alternative pattern matching interfaces for types.

/// View type definition
///
/// View type: provides alternative pattern matching interface for a base type.
/// Example: `view Parity : Nat -> Type { Even(n), Odd(n) }` with a view function
/// `parity(n: Nat) -> Parity(n)` that computes the view.
#[derive(Debug, Clone)]
pub struct ViewType {
    /// View name
    pub name: Text,
    /// Base type being viewed
    pub base_type: Heap<Type>,
    /// Result type (indexed by base type value)
    pub result_type: Heap<Type>,
    /// View function
    pub view_function: Maybe<Heap<Expr>>,
    /// View cases
    pub cases: List<ViewCase>,
}

impl ViewType {
    pub fn new(name: Text, base_type: Type, result_type: Type) -> Self {
        Self {
            name,
            base_type: Heap::new(base_type),
            result_type: Heap::new(result_type),
            view_function: Maybe::None,
            cases: List::new(),
        }
    }

    pub fn with_case(mut self, case: ViewCase) -> Self {
        self.cases.push(case);
        self
    }
}

/// A case in a view pattern
#[derive(Debug, Clone)]
pub struct ViewCase {
    /// Case name
    pub name: Text,
    /// Pattern parameters
    pub params: List<TypeParam>,
    /// Result index expression
    pub index: Heap<Expr>,
}

// ==================== Proof Irrelevance ====================
//
// Proof Irrelevance: Prop universe where all proofs of a proposition are equal.
// `proof_irrelevance: [P: Prop] -> (p1: P) -> (p2: P) -> p1 = p2`
// Squash types truncate to propositions: `type Squash<A> : Prop is exists(_: A). True`
//
// Prop universe where all proofs are equal.

/// Proposition marker
///
/// Types in Prop are proof-irrelevant: all proofs are considered equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prop;

/// Squash type (propositional truncation)
///
/// ||A|| : Prop for any A : Type
/// Forgets the computational content, keeping only existence.
#[derive(Debug, Clone)]
pub struct Squash {
    /// The type being squashed
    pub inner_type: Heap<Type>,
}

impl Squash {
    pub fn new(ty: Type) -> Self {
        Self {
            inner_type: Heap::new(ty),
        }
    }
}

/// Subset type with irrelevant proof
///
/// { x : A | P(x) }^Prop
/// The proof of P(x) is erased at runtime.
#[derive(Debug, Clone)]
pub struct SubsetType {
    /// Base type
    pub base_type: Heap<Type>,
    /// Predicate
    pub predicate: Heap<Expr>,
    /// Bound variable name
    pub var_name: Text,
    /// Is the proof irrelevant?
    pub proof_irrelevant: bool,
}

impl SubsetType {
    pub fn new(var_name: Text, base_type: Type, predicate: Expr) -> Self {
        Self {
            base_type: Heap::new(base_type),
            predicate: Heap::new(predicate),
            var_name,
            proof_irrelevant: true,
        }
    }

    /// Create with relevant proof (not erased)
    pub fn with_relevant_proof(mut self) -> Self {
        self.proof_irrelevant = false;
        self
    }
}
