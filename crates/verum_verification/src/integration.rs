//! Integration with Type System, SMT, and Codegen
//!
//! Provides integration points for gradual verification with:
//! - verum_types: Refinement type checking
//! - verum_smt: SMT-based verification
//! - verum_codegen: Code generation with verification info
//!
//! Integration points:
//! - verum_types: refinement type constraints provide additional SMT assumptions
//! - verum_smt: Z3/CVC5 backend for proving VCs and refinement subsumption
//! - verum_codegen: verification results guide check elimination in AOT code

use crate::Error;
use crate::context::VerificationContext;
use crate::level::{VerificationLevel, VerificationMode};
use verum_smt::{VerifyMode, verify_refinement};
use verum_types::{RefinementChecker, Type};

/// Integration with verum_types type system
#[derive(Debug)]
pub struct TypeSystemIntegration;

impl TypeSystemIntegration {
    /// Check if a type requires verification
    ///
    /// Returns true if the type contains refinement predicates that need
    /// SMT verification. This recursively checks nested types.
    pub fn requires_verification(ty: &Type) -> bool {
        Self::has_refinement(ty)
    }

    /// Recursively check if a type contains refinement predicates
    fn has_refinement(ty: &Type) -> bool {
        match ty {
            // Refined types always require verification
            Type::Refined { .. } => true,

            // Check nested types
            Type::Function {
                params,
                return_type,
                ..
            } => params.iter().any(Self::has_refinement) || Self::has_refinement(return_type),
            Type::Tuple(types) => types.iter().any(Self::has_refinement),
            Type::Array { element, .. } => Self::has_refinement(element),
            Type::Slice { element } => Self::has_refinement(element),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. }
            | Type::GenRef { inner } => Self::has_refinement(inner),
            Type::Named { args, .. } | Type::Generic { args, .. } => {
                args.iter().any(Self::has_refinement)
            }
            Type::Exists { body, .. } | Type::Forall { body, .. } => Self::has_refinement(body),
            Type::Record(fields) | Type::Variant(fields) => {
                fields.values().any(Self::has_refinement)
            }
            Type::ExtensibleRecord { fields, .. } => fields.values().any(Self::has_refinement),

            // Primitive types don't require verification
            Type::Unit
            | Type::Never
            | Type::Unknown
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text
            | Type::Var(_)
            | Type::Lifetime { .. }
            | Type::TypeConstructor { .. } => false,

            // Meta types and special async types
            Type::Meta { ty, .. } => Self::has_refinement(ty),
            Type::Future { output } => Self::has_refinement(output),
            Type::Generator {
                yield_ty,
                return_ty,
            } => Self::has_refinement(yield_ty) || Self::has_refinement(return_ty),
            Type::Tensor { element, .. } => Self::has_refinement(element),
            Type::TypeApp { constructor, args } => {
                Self::has_refinement(constructor) || args.iter().any(Self::has_refinement)
            }

            // Dependent types - check nested types recursively
            Type::Pi {
                param_type,
                return_type,
                ..
            } => Self::has_refinement(param_type) || Self::has_refinement(return_type),
            Type::Sigma {
                fst_type, snd_type, ..
            } => Self::has_refinement(fst_type) || Self::has_refinement(snd_type),
            Type::Eq { ty, .. } => {
                // Equality types themselves may contain refinements in the base type
                Self::has_refinement(ty)
            }
            Type::Inductive {
                params,
                indices,
                constructors,
                ..
            } => {
                // Check parameters and indices for refinements
                params.iter().any(|(_, ty)| Self::has_refinement(ty))
                    || indices.iter().any(|(_, ty)| Self::has_refinement(ty))
                    || constructors.iter().any(|c| {
                        c.args.iter().any(|arg| Self::has_refinement(arg))
                            || Self::has_refinement(&c.return_type)
                    })
            }
            Type::Coinductive {
                params,
                destructors,
                ..
            } => {
                // Check parameters and destructors for refinements
                params.iter().any(|(_, ty)| Self::has_refinement(ty))
                    || destructors
                        .iter()
                        .any(|d| Self::has_refinement(&d.result_type))
            }
            Type::HigherInductive {
                params,
                point_constructors,
                path_constructors,
                ..
            } => {
                // Check parameters and constructors for refinements
                params.iter().any(|(_, ty)| Self::has_refinement(ty))
                    || point_constructors.iter().any(|c| {
                        c.args.iter().any(|arg| Self::has_refinement(arg))
                            || Self::has_refinement(&c.return_type)
                    })
                    || path_constructors
                        .iter()
                        .any(|pc| pc.args.iter().any(|arg| Self::has_refinement(arg)))
            }
            Type::Quantified { inner, .. } => {
                // Check the inner type for refinements
                Self::has_refinement(inner)
            }

            // Universe and Prop types are meta-level constructs
            // They don't contain runtime refinements that need verification
            Type::Universe { .. } | Type::Prop => false,

            // Placeholder types - should be resolved before verification
            Type::Placeholder { .. } => false,

            // CapabilityRestricted types - check base type for refinements
            Type::CapabilityRestricted { base, .. } => Self::has_refinement(base),

            // Dynamic protocol types - check associated type bindings for refinements
            Type::DynProtocol { bindings, .. } => bindings.values().any(Self::has_refinement),
        }
    }

    /// Get recommended verification level for a type
    pub fn recommend_level(ty: &Type) -> VerificationLevel {
        if Self::requires_verification(ty) {
            VerificationLevel::Static
        } else {
            VerificationLevel::Runtime
        }
    }
}

/// Integration with verum_smt SMT solver
#[derive(Debug)]
pub struct SmtIntegration;

impl SmtIntegration {
    /// Convert verification level to SMT verify mode
    pub fn to_smt_mode(level: VerificationLevel) -> VerifyMode {
        match level {
            VerificationLevel::Runtime => VerifyMode::Runtime,
            VerificationLevel::Static => VerifyMode::Auto,
            VerificationLevel::Proof => VerifyMode::Proof,
        }
    }
}

/// Integration with verum_codegen code generation
#[derive(Debug)]
pub struct CodegenIntegration;

impl CodegenIntegration {
    /// Determine if runtime checks should be emitted
    pub fn emit_runtime_checks(level: VerificationLevel, proven_safe: bool) -> bool {
        match level {
            VerificationLevel::Runtime => true,
            VerificationLevel::Static => !proven_safe,
            VerificationLevel::Proof => !proven_safe,
        }
    }

    /// Get optimization level for verification mode
    pub fn optimization_level(level: VerificationLevel) -> u8 {
        match level {
            VerificationLevel::Runtime => 1,
            VerificationLevel::Static => 2,
            VerificationLevel::Proof => 3,
        }
    }
}

// =============================================================================
// Hoare Logic Z3 Integration
// =============================================================================

use crate::hoare_logic::{Command, HoareTriple, WPError};
use crate::vcgen::{Formula, SmtBinOp, SmtExpr, SmtUnOp, Variable};
use crate::vcgen::{VCResult, VerificationCondition};
use verum_smt::context::Context as SmtContext;
use verum_common::{Map, Text};

/// Hoare logic verification using Z3 SMT solver
///
/// Provides production-grade integration between Hoare logic verification
/// and the Z3 SMT solver for automated theorem proving.
#[derive(Debug)]
pub struct HoareZ3Verifier<'ctx> {
    /// Z3 context for SMT solving
    context: &'ctx SmtContext,
    /// Timeout for verification in milliseconds
    timeout_ms: u32,
    /// Enable proof generation for certification
    generate_proofs: bool,
}

impl<'ctx> HoareZ3Verifier<'ctx> {
    /// Create a new Hoare logic verifier with Z3 backend
    pub fn new(context: &'ctx SmtContext) -> Self {
        Self {
            context,
            timeout_ms: 30000, // 30 second default timeout
            generate_proofs: false,
        }
    }

    /// Set verification timeout in milliseconds
    pub fn with_timeout(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Enable proof generation for certification
    pub fn with_proofs(mut self) -> Self {
        self.generate_proofs = true;
        self
    }

    /// Verify a Hoare triple using Z3
    ///
    /// Returns Ok(true) if the triple is valid, Ok(false) if invalid with counterexample,
    /// or Err if verification fails or times out.
    pub fn verify_triple(&self, triple: &HoareTriple) -> Result<HoareVerificationResult, WPError> {
        use crate::hoare_logic::WPCalculator;

        // Compute weakest precondition
        let wp_calc = WPCalculator::new();
        let wp = wp_calc.wp(&triple.command, &triple.postcondition)?;

        // Create implication: precondition => wp
        let vc_formula = Formula::Implies(Box::new(triple.precondition.clone()), Box::new(wp));

        // Verify using Z3
        self.verify_formula(&vc_formula)
    }

    /// Verify a formula using Z3
    ///
    /// The formula is valid if its negation is unsatisfiable.
    pub fn verify_formula(&self, formula: &Formula) -> Result<HoareVerificationResult, WPError> {
        let solver = self.context.solver();

        // Apply timeout configuration
        let mut params = z3::Params::new();
        params.set_u32("timeout", self.timeout_ms);
        solver.set_params(&params);

        // Collect free variables and declare them
        let free_vars = self.collect_free_variables(formula);
        let var_decls = self.declare_variables(&free_vars);

        // Translate formula to Z3
        let z3_formula = self.translate_formula(formula, &var_decls)?;

        // Assert negation (valid iff negation is UNSAT)
        solver.assert(z3_formula.not());

        // Check satisfiability
        match solver.check() {
            z3::SatResult::Unsat => {
                // Formula is valid (negation is UNSAT)
                let mut result = HoareVerificationResult::valid();

                // Extract proof if requested
                if self.generate_proofs {
                    if let Some(proof) = solver.get_proof() {
                        result = result.with_proof(format!("{:?}", proof));
                    }
                }

                Ok(result)
            }
            z3::SatResult::Sat => {
                // Formula is invalid - extract counterexample
                if let Some(model) = solver.get_model() {
                    let counterexample = self.extract_counterexample(&model, &var_decls);
                    Ok(HoareVerificationResult::invalid(counterexample))
                } else {
                    Ok(HoareVerificationResult::invalid(Map::new()))
                }
            }
            z3::SatResult::Unknown => {
                // Solver returned unknown (likely timeout)
                let reason = solver
                    .get_reason_unknown()
                    .map(|s| Text::from(s.to_string()))
                    .unwrap_or_else(|| Text::from("unknown"));
                Err(WPError::Unknown {
                    reason,
                    location: None,
                })
            }
        }
    }

    /// Collect all free variables from a formula
    fn collect_free_variables(&self, formula: &Formula) -> Vec<(Text, VarSort)> {
        let mut vars = Vec::new();
        self.collect_vars_formula(formula, &mut vars);
        vars
    }

    fn collect_vars_formula(&self, formula: &Formula, vars: &mut Vec<(Text, VarSort)>) {
        match formula {
            Formula::Var(v) => {
                let name = v.smtlib_name();
                if !vars.iter().any(|(n, _)| n == &name) {
                    vars.push((name, VarSort::Bool));
                }
            }
            Formula::Not(inner) => self.collect_vars_formula(inner, vars),
            Formula::And(formulas) | Formula::Or(formulas) => {
                for f in formulas.iter() {
                    self.collect_vars_formula(f, vars);
                }
            }
            Formula::Implies(ante, cons) | Formula::Iff(ante, cons) => {
                self.collect_vars_formula(ante, vars);
                self.collect_vars_formula(cons, vars);
            }
            Formula::Forall(bound, inner) | Formula::Exists(bound, inner) => {
                // Don't collect bound variables
                self.collect_vars_formula(inner, vars);
            }
            Formula::Eq(left, right)
            | Formula::Ne(left, right)
            | Formula::Lt(left, right)
            | Formula::Le(left, right)
            | Formula::Gt(left, right)
            | Formula::Ge(left, right) => {
                self.collect_vars_expr(left, vars);
                self.collect_vars_expr(right, vars);
            }
            Formula::Predicate(_, args) => {
                for arg in args.iter() {
                    self.collect_vars_expr(arg, vars);
                }
            }
            Formula::Let(_, bound_expr, body) => {
                self.collect_vars_expr(bound_expr, vars);
                self.collect_vars_formula(body, vars);
            }
            Formula::True | Formula::False => {}
        }
    }

    fn collect_vars_expr(&self, expr: &SmtExpr, vars: &mut Vec<(Text, VarSort)>) {
        match expr {
            SmtExpr::Var(v) => {
                let name = v.smtlib_name();
                if !vars.iter().any(|(n, _)| n == &name) {
                    // Infer sort from variable type if available
                    let sort = match &v.ty {
                        Some(ty) => self.var_type_to_sort(ty),
                        None => VarSort::Int, // Default to Int
                    };
                    vars.push((name, sort));
                }
            }
            SmtExpr::BinOp(_, left, right) => {
                self.collect_vars_expr(left, vars);
                self.collect_vars_expr(right, vars);
            }
            SmtExpr::UnOp(_, inner) => self.collect_vars_expr(inner, vars),
            SmtExpr::Apply(_, args) => {
                for arg in args.iter() {
                    self.collect_vars_expr(arg, vars);
                }
            }
            SmtExpr::Select(arr, idx) => {
                self.collect_vars_expr(arr, vars);
                self.collect_vars_expr(idx, vars);
            }
            SmtExpr::Store(arr, idx, val) => {
                self.collect_vars_expr(arr, vars);
                self.collect_vars_expr(idx, vars);
                self.collect_vars_expr(val, vars);
            }
            SmtExpr::Ite(cond, then_e, else_e) => {
                self.collect_vars_formula(cond, vars);
                self.collect_vars_expr(then_e, vars);
                self.collect_vars_expr(else_e, vars);
            }
            SmtExpr::Let(_, bound, body) => {
                self.collect_vars_expr(bound, vars);
                self.collect_vars_expr(body, vars);
            }
            SmtExpr::IntConst(_)
            | SmtExpr::BoolConst(_)
            | SmtExpr::RealConst(_)
            | SmtExpr::BitVecConst(_, _) => {}
        }
    }

    fn var_type_to_sort(&self, ty: &crate::vcgen::VarType) -> VarSort {
        use crate::vcgen::VarType;
        match ty {
            VarType::Int => VarSort::Int,
            VarType::Bool => VarSort::Bool,
            VarType::Real => VarSort::Real,
            VarType::BitVec(w) => VarSort::BitVec(*w),
            VarType::Array(idx, elem) => VarSort::Array(
                Box::new(self.var_type_to_sort(idx)),
                Box::new(self.var_type_to_sort(elem)),
            ),
            VarType::Sort(_) => VarSort::Int, // Default uninterpreted sorts to Int
        }
    }

    /// Declare Z3 variables for all free variables
    fn declare_variables(&self, vars: &[(Text, VarSort)]) -> Map<Text, z3::ast::Dynamic> {
        let mut decls = Map::new();

        for (name, sort) in vars {
            let z3_var: z3::ast::Dynamic = match sort {
                VarSort::Int => z3::ast::Int::new_const(name.as_str()).into(),
                VarSort::Bool => z3::ast::Bool::new_const(name.as_str()).into(),
                VarSort::Real => z3::ast::Real::new_const(name.as_str()).into(),
                VarSort::BitVec(w) => z3::ast::BV::new_const(name.as_str(), *w).into(),
                VarSort::Array(idx_sort, elem_sort) => {
                    let idx_z3 = self.sort_to_z3(idx_sort);
                    let elem_z3 = self.sort_to_z3(elem_sort);
                    z3::ast::Array::new_const(name.as_str(), &idx_z3, &elem_z3).into()
                }
            };
            decls.insert(name.clone(), z3_var);
        }

        decls
    }

    fn sort_to_z3(&self, sort: &VarSort) -> z3::Sort {
        match sort {
            VarSort::Int => z3::Sort::int(),
            VarSort::Bool => z3::Sort::bool(),
            VarSort::Real => z3::Sort::real(),
            VarSort::BitVec(w) => z3::Sort::bitvector(*w),
            VarSort::Array(idx, elem) => {
                let idx_sort = self.sort_to_z3(idx);
                let elem_sort = self.sort_to_z3(elem);
                z3::Sort::array(&idx_sort, &elem_sort)
            }
        }
    }

    /// Translate a formula to Z3
    fn translate_formula(
        &self,
        formula: &Formula,
        vars: &Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Bool, WPError> {
        match formula {
            Formula::True => Ok(z3::ast::Bool::from_bool(true)),
            Formula::False => Ok(z3::ast::Bool::from_bool(false)),
            Formula::Var(v) => {
                let name = v.smtlib_name();
                if let Some(z3_var) = vars.get(&name) {
                    z3_var.as_bool().ok_or_else(|| {
                        WPError::TypeError(Text::from(format!("Variable {} is not boolean", name)))
                    })
                } else {
                    // Create new boolean variable
                    Ok(z3::ast::Bool::new_const(name.as_str()))
                }
            }
            Formula::Not(inner) => {
                let z3_inner = self.translate_formula(inner, vars)?;
                Ok(z3_inner.not())
            }
            Formula::And(formulas) => {
                let z3_formulas: Result<Vec<_>, _> = formulas
                    .iter()
                    .map(|f| self.translate_formula(f, vars))
                    .collect();
                let z3_formulas = z3_formulas?;
                let refs: Vec<_> = z3_formulas.iter().collect();
                Ok(z3::ast::Bool::and(&refs))
            }
            Formula::Or(formulas) => {
                let z3_formulas: Result<Vec<_>, _> = formulas
                    .iter()
                    .map(|f| self.translate_formula(f, vars))
                    .collect();
                let z3_formulas = z3_formulas?;
                let refs: Vec<_> = z3_formulas.iter().collect();
                Ok(z3::ast::Bool::or(&refs))
            }
            Formula::Implies(ante, cons) => {
                let z3_ante = self.translate_formula(ante, vars)?;
                let z3_cons = self.translate_formula(cons, vars)?;
                Ok(z3_ante.implies(&z3_cons))
            }
            Formula::Iff(left, right) => {
                let z3_left = self.translate_formula(left, vars)?;
                let z3_right = self.translate_formula(right, vars)?;
                Ok(z3_left.iff(&z3_right))
            }
            Formula::Forall(bound_vars, inner) => {
                // Create bound variables
                let mut new_vars = vars.clone();
                let mut bounds: Vec<z3::ast::Dynamic> = Vec::new();

                for bv in bound_vars.iter() {
                    let name = bv.smtlib_name();
                    let sort = match &bv.ty {
                        Some(ty) => self.var_type_to_sort(ty),
                        None => VarSort::Int,
                    };
                    let z3_var: z3::ast::Dynamic = match sort {
                        VarSort::Int => z3::ast::Int::new_const(name.as_str()).into(),
                        VarSort::Bool => z3::ast::Bool::new_const(name.as_str()).into(),
                        VarSort::Real => z3::ast::Real::new_const(name.as_str()).into(),
                        _ => z3::ast::Int::new_const(name.as_str()).into(),
                    };
                    bounds.push(z3_var.clone());
                    new_vars.insert(name, z3_var);
                }

                let z3_inner = self.translate_formula(inner, &new_vars)?;
                let bound_refs: Vec<&dyn z3::ast::Ast> =
                    bounds.iter().map(|b| b as &dyn z3::ast::Ast).collect();
                Ok(z3::ast::forall_const(&bound_refs, &[], &z3_inner))
            }
            Formula::Exists(bound_vars, inner) => {
                let mut new_vars = vars.clone();
                let mut bounds: Vec<z3::ast::Dynamic> = Vec::new();

                for bv in bound_vars.iter() {
                    let name = bv.smtlib_name();
                    let z3_var: z3::ast::Dynamic = z3::ast::Int::new_const(name.as_str()).into();
                    bounds.push(z3_var.clone());
                    new_vars.insert(name, z3_var);
                }

                let z3_inner = self.translate_formula(inner, &new_vars)?;
                let bound_refs: Vec<&dyn z3::ast::Ast> =
                    bounds.iter().map(|b| b as &dyn z3::ast::Ast).collect();
                Ok(z3::ast::exists_const(&bound_refs, &[], &z3_inner))
            }
            Formula::Eq(left, right) => {
                let z3_left = self.translate_expr(left, vars)?;
                let z3_right = self.translate_expr(right, vars)?;
                Ok(z3_left.eq(&z3_right))
            }
            Formula::Ne(left, right) => {
                let z3_left = self.translate_expr(left, vars)?;
                let z3_right = self.translate_expr(right, vars)?;
                Ok(z3_left.eq(&z3_right).not())
            }
            Formula::Lt(left, right) => {
                let z3_left = self.translate_expr_as_int(left, vars)?;
                let z3_right = self.translate_expr_as_int(right, vars)?;
                Ok(z3_left.lt(&z3_right))
            }
            Formula::Le(left, right) => {
                let z3_left = self.translate_expr_as_int(left, vars)?;
                let z3_right = self.translate_expr_as_int(right, vars)?;
                Ok(z3_left.le(&z3_right))
            }
            Formula::Gt(left, right) => {
                let z3_left = self.translate_expr_as_int(left, vars)?;
                let z3_right = self.translate_expr_as_int(right, vars)?;
                Ok(z3_left.gt(&z3_right))
            }
            Formula::Ge(left, right) => {
                let z3_left = self.translate_expr_as_int(left, vars)?;
                let z3_right = self.translate_expr_as_int(right, vars)?;
                Ok(z3_left.ge(&z3_right))
            }
            Formula::Predicate(name, args) => {
                // Handle special predicates
                if name.as_str() == "is_true" && args.len() == 1 {
                    let z3_arg = self.translate_expr(&args[0], vars)?;
                    return z3_arg.as_bool().ok_or_else(|| {
                        WPError::TypeError(Text::from("is_true requires boolean argument"))
                    });
                }

                // General predicate - create uninterpreted function
                let arg_sorts: Vec<_> = args.iter().map(|_| z3::Sort::int()).collect();
                let arg_sort_refs: Vec<_> = arg_sorts.iter().collect();
                let func_decl = z3::FuncDecl::new(name.as_str(), &arg_sort_refs, &z3::Sort::bool());

                let z3_args: Result<Vec<_>, _> =
                    args.iter().map(|a| self.translate_expr(a, vars)).collect();
                let z3_args = z3_args?;
                let arg_refs: Vec<_> = z3_args.iter().map(|a| a as &dyn z3::ast::Ast).collect();

                func_decl.apply(&arg_refs).as_bool().ok_or_else(|| {
                    WPError::TypeError(Text::from("Predicate did not return boolean"))
                })
            }
            Formula::Let(bound_var, bound_expr, body) => {
                let z3_bound = self.translate_expr(bound_expr, vars)?;
                let mut new_vars = vars.clone();
                new_vars.insert(bound_var.smtlib_name(), z3_bound);
                self.translate_formula(body, &new_vars)
            }
        }
    }

    /// Translate an expression to Z3
    fn translate_expr(
        &self,
        expr: &SmtExpr,
        vars: &Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Dynamic, WPError> {
        match expr {
            SmtExpr::Var(v) => {
                let name = v.smtlib_name();
                if let Some(z3_var) = vars.get(&name) {
                    Ok(z3_var.clone())
                } else {
                    // Create new integer variable by default
                    Ok(z3::ast::Int::new_const(name.as_str()).into())
                }
            }
            SmtExpr::IntConst(n) => Ok(z3::ast::Int::from_i64(*n).into()),
            SmtExpr::BoolConst(b) => Ok(z3::ast::Bool::from_bool(*b).into()),
            SmtExpr::RealConst(r) => {
                // Convert f64 to rational approximation
                let (num, denom) = float_to_rational(*r);
                Ok(z3::ast::Real::from_rational(num as i64, denom as i64).into())
            }
            SmtExpr::BitVecConst(v, w) => Ok(z3::ast::BV::from_u64(*v, *w).into()),
            SmtExpr::BinOp(op, left, right) => {
                let z3_left = self.translate_expr_as_int(left, vars)?;
                let z3_right = self.translate_expr_as_int(right, vars)?;

                let result: z3::ast::Int = match op {
                    SmtBinOp::Add => z3::ast::Int::add(&[&z3_left, &z3_right]),
                    SmtBinOp::Sub => z3::ast::Int::sub(&[&z3_left, &z3_right]),
                    SmtBinOp::Mul => z3::ast::Int::mul(&[&z3_left, &z3_right]),
                    SmtBinOp::Div => z3_left.div(&z3_right),
                    SmtBinOp::Mod => z3_left.rem(&z3_right),
                    SmtBinOp::Pow => {
                        // Z3 doesn't have direct power, use uninterpreted function
                        let pow_decl = z3::FuncDecl::new(
                            "pow",
                            &[&z3::Sort::int(), &z3::Sort::int()],
                            &z3::Sort::int(),
                        );
                        let result = pow_decl.apply(&[&z3_left, &z3_right]);
                        return Ok(result);
                    }
                    SmtBinOp::Select => {
                        // Array select - translate differently
                        let arr = self.translate_expr(left, vars)?;
                        let idx = self.translate_expr(right, vars)?;
                        if let (Some(z3_arr), Some(z3_idx)) = (arr.as_array(), idx.as_int()) {
                            return Ok(z3_arr.select(&z3_idx));
                        }
                        return Err(WPError::TypeError(Text::from(
                            "Select requires array and integer index",
                        )));
                    }
                };
                Ok(result.into())
            }
            SmtExpr::UnOp(op, inner) => {
                match op {
                    SmtUnOp::Neg => {
                        let z3_inner = self.translate_expr_as_int(inner, vars)?;
                        Ok(z3_inner.unary_minus().into())
                    }
                    SmtUnOp::Abs => {
                        let z3_inner = self.translate_expr_as_int(inner, vars)?;
                        let zero = z3::ast::Int::from_i64(0);
                        let is_neg = z3_inner.lt(&zero);
                        let neg = z3_inner.unary_minus();
                        Ok(is_neg.ite(&neg, &z3_inner).into())
                    }
                    SmtUnOp::Deref | SmtUnOp::Len | SmtUnOp::GetVariantValue => {
                        // These are uninterpreted functions
                        let name = match op {
                            SmtUnOp::Deref => "deref",
                            SmtUnOp::Len => "len",
                            SmtUnOp::GetVariantValue => "get_variant_value",
                            _ => unreachable!(),
                        };
                        let func = z3::FuncDecl::new(name, &[&z3::Sort::int()], &z3::Sort::int());
                        let z3_inner = self.translate_expr(inner, vars)?;
                        Ok(func.apply(&[&z3_inner]))
                    }
                }
            }
            SmtExpr::Apply(name, args) => {
                // Uninterpreted function application
                let arg_sorts: Vec<_> = args.iter().map(|_| z3::Sort::int()).collect();
                let arg_sort_refs: Vec<_> = arg_sorts.iter().collect();
                let func_decl = z3::FuncDecl::new(name.as_str(), &arg_sort_refs, &z3::Sort::int());

                let z3_args: Result<Vec<_>, _> =
                    args.iter().map(|a| self.translate_expr(a, vars)).collect();
                let z3_args = z3_args?;
                let arg_refs: Vec<_> = z3_args.iter().map(|a| a as &dyn z3::ast::Ast).collect();

                Ok(func_decl.apply(&arg_refs))
            }
            SmtExpr::Select(arr, idx) => {
                let z3_arr = self.translate_expr(arr, vars)?;
                let z3_idx = self.translate_expr(idx, vars)?;

                if let (Some(arr_val), Some(idx_val)) = (z3_arr.as_array(), z3_idx.as_int()) {
                    Ok(arr_val.select(&idx_val))
                } else {
                    Err(WPError::TypeError(Text::from(
                        "Select requires array and integer index",
                    )))
                }
            }
            SmtExpr::Store(arr, idx, val) => {
                let z3_arr = self.translate_expr(arr, vars)?;
                let z3_idx = self.translate_expr(idx, vars)?;
                let z3_val = self.translate_expr(val, vars)?;

                if let (Some(arr_val), Some(idx_val)) = (z3_arr.as_array(), z3_idx.as_int()) {
                    Ok(arr_val.store(&idx_val, &z3_val).into())
                } else {
                    Err(WPError::TypeError(Text::from(
                        "Store requires array and integer index",
                    )))
                }
            }
            SmtExpr::Ite(cond, then_e, else_e) => {
                let z3_cond = self.translate_formula(cond, vars)?;
                let z3_then = self.translate_expr(then_e, vars)?;
                let z3_else = self.translate_expr(else_e, vars)?;

                // Use ite on integers
                if let (Some(then_int), Some(else_int)) = (z3_then.as_int(), z3_else.as_int()) {
                    Ok(z3_cond.ite(&then_int, &else_int).into())
                } else {
                    Err(WPError::TypeError(Text::from(
                        "ITE branches must have same type",
                    )))
                }
            }
            SmtExpr::Let(bound_var, bound_expr, body) => {
                let z3_bound = self.translate_expr(bound_expr, vars)?;
                let mut new_vars = vars.clone();
                new_vars.insert(bound_var.smtlib_name(), z3_bound);
                self.translate_expr(body, &new_vars)
            }
        }
    }

    /// Translate expression, ensuring it's an integer
    fn translate_expr_as_int(
        &self,
        expr: &SmtExpr,
        vars: &Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Int, WPError> {
        let z3_expr = self.translate_expr(expr, vars)?;
        z3_expr
            .as_int()
            .ok_or_else(|| WPError::TypeError(Text::from("Expected integer expression")))
    }

    /// Extract counterexample from Z3 model
    fn extract_counterexample(
        &self,
        model: &z3::Model,
        vars: &Map<Text, z3::ast::Dynamic>,
    ) -> Map<Text, Text> {
        let mut result = Map::new();

        for (name, z3_var) in vars.iter() {
            if let Some(val) = model.eval(z3_var, true) {
                result.insert(name.clone(), Text::from(format!("{}", val)));
            }
        }

        result
    }
}

/// Helper function to convert f64 to rational approximation
fn float_to_rational(f: f64) -> (i32, i32) {
    // Simple rational approximation for f64
    const PRECISION: i32 = 1000000;
    let scaled = (f * PRECISION as f64).round() as i32;
    let gcd = gcd(scaled.abs(), PRECISION);
    (scaled / gcd, PRECISION / gcd)
}

fn gcd(a: i32, b: i32) -> i32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Variable sort for Z3 translation
#[derive(Debug, Clone)]
pub enum VarSort {
    Int,
    Bool,
    Real,
    BitVec(u32),
    Array(Box<VarSort>, Box<VarSort>),
}

/// Result of Hoare logic verification
#[derive(Debug, Clone)]
pub struct HoareVerificationResult {
    /// Whether the triple is valid
    pub valid: bool,
    /// Counterexample if invalid
    pub counterexample: Option<Map<Text, Text>>,
    /// Proof string if generated
    pub proof: Option<Text>,
}

impl HoareVerificationResult {
    /// Create a valid result
    pub fn valid() -> Self {
        Self {
            valid: true,
            counterexample: None,
            proof: None,
        }
    }

    /// Create an invalid result with counterexample
    pub fn invalid(counterexample: Map<Text, Text>) -> Self {
        Self {
            valid: false,
            counterexample: Some(counterexample),
            proof: None,
        }
    }

    /// Add proof to result
    pub fn with_proof(mut self, proof: impl Into<Text>) -> Self {
        self.proof = Some(proof.into());
        self
    }
}

// =============================================================================
// Separation Logic Z3 Integration
// =============================================================================

use crate::separation_logic::{Address, Heap as SepHeap, HeapCommand, SepProp, Value};

/// Separation logic verification using Z3 with array theory
///
/// Models heap as Z3 arrays for efficient verification of heap properties.
#[derive(Debug)]
pub struct SeparationLogicZ3Verifier<'ctx> {
    /// Z3 context
    context: &'ctx SmtContext,
    /// Timeout in milliseconds
    timeout_ms: u32,
}

impl<'ctx> SeparationLogicZ3Verifier<'ctx> {
    /// Create a new separation logic verifier
    pub fn new(context: &'ctx SmtContext) -> Self {
        Self {
            context,
            timeout_ms: 30000,
        }
    }

    /// Set verification timeout
    pub fn with_timeout(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Verify a separation logic triple using Z3
    ///
    /// Uses array theory to model the heap and verifies:
    /// {pre} cmd {post}
    pub fn verify_triple(
        &self,
        pre: &SepProp,
        cmd: &HeapCommand,
        post: &SepProp,
    ) -> Result<SepLogicVerificationResult, WPError> {
        let solver = self.context.solver();

        // Apply timeout
        let mut params = z3::Params::new();
        params.set_u32("timeout", self.timeout_ms);
        solver.set_params(&params);

        // Model heap as array from Int (address) to Int (value)
        let _heap_sort = z3::Sort::array(&z3::Sort::int(), &z3::Sort::int());
        let heap_pre = z3::ast::Array::new_const("heap_pre", &z3::Sort::int(), &z3::Sort::int());
        let heap_post = z3::ast::Array::new_const("heap_post", &z3::Sort::int(), &z3::Sort::int());

        // Domain tracking for separation (which addresses are valid)
        let domain_pre =
            z3::ast::Array::new_const("domain_pre", &z3::Sort::int(), &z3::Sort::bool());
        let domain_post =
            z3::ast::Array::new_const("domain_post", &z3::Sort::int(), &z3::Sort::bool());

        // Encode precondition
        let pre_formula = self.encode_sep_prop(pre, &heap_pre, &domain_pre)?;
        solver.assert(&pre_formula);

        // Encode command semantics
        let (heap_after_cmd, domain_after_cmd) =
            self.encode_command(cmd, &heap_pre, &domain_pre)?;

        // Heap and domain after command equal post heap/domain
        solver.assert(heap_post.eq(&heap_after_cmd));
        solver.assert(domain_post.eq(&domain_after_cmd));

        // Encode postcondition negation (we want to prove post holds)
        let post_formula = self.encode_sep_prop(post, &heap_post, &domain_post)?;
        solver.assert(post_formula.not());

        // Check if UNSAT (meaning postcondition always holds)
        match solver.check() {
            z3::SatResult::Unsat => Ok(SepLogicVerificationResult::valid()),
            z3::SatResult::Sat => {
                if let Some(model) = solver.get_model() {
                    let counterexample =
                        self.extract_heap_counterexample(&model, &heap_pre, &heap_post);
                    Ok(SepLogicVerificationResult::invalid(counterexample))
                } else {
                    Ok(SepLogicVerificationResult::invalid(
                        HeapCounterexample::default(),
                    ))
                }
            }
            z3::SatResult::Unknown => {
                let reason = solver
                    .get_reason_unknown()
                    .map(|s| Text::from(s.to_string()))
                    .unwrap_or_else(|| Text::from("unknown"));
                Err(WPError::Unknown {
                    reason,
                    location: None,
                })
            }
        }
    }

    /// Encode separation logic proposition as Z3 formula
    fn encode_sep_prop(
        &self,
        prop: &SepProp,
        heap: &z3::ast::Array,
        domain: &z3::ast::Array,
    ) -> Result<z3::ast::Bool, WPError> {
        match prop {
            SepProp::Emp => {
                // Empty heap: domain is all false
                let addr = z3::ast::Int::new_const("_addr");
                let domain_at_addr = domain.select(&addr);
                let is_false = domain_at_addr
                    .as_bool()
                    .ok_or_else(|| WPError::TypeError(Text::from("Domain should be boolean")))?;
                Ok(z3::ast::forall_const(
                    &[&z3::ast::Dynamic::from(addr.clone())],
                    &[],
                    &is_false.not().not(), // forall addr. !domain[addr]
                ))
            }
            SepProp::PointsTo(addr, val) => {
                // addr |-> val: heap[addr] = val and domain[addr] = true
                let z3_addr = self.encode_address(addr)?;
                let z3_val = self.encode_value(val)?;

                let heap_val = heap.select(&z3_addr);
                let domain_val = domain.select(&z3_addr);

                let heap_eq = heap_val.eq(z3::ast::Dynamic::from(z3_val));
                let in_domain = domain_val
                    .as_bool()
                    .ok_or_else(|| WPError::TypeError(Text::from("Domain should be boolean")))?;

                Ok(z3::ast::Bool::and(&[&heap_eq, &in_domain]))
            }
            SepProp::SeparatingConj(p, q) => {
                // p * q: separating conjunction - disjoint heaps that merge to parent
                //
                // Separation logic: P * Q means heap splits into disjoint parts
                // satisfying P and Q respectively. The frame rule {P} c {Q}
                // implies {P * R} c {Q * R} when c doesn't touch R.
                //
                // Semantics: (P * Q)(h) iff exists h1, h2.
                //   - h = h1 + h2 (heap merge/disjoint union)
                //   - dom(h1) ∩ dom(h2) = {} (disjointness)
                //   - P(h1) and Q(h2)
                //
                // We encode this by:
                // 1. Creating fresh sub-heaps h1, h2 with domains d1, d2
                // 2. Asserting P holds on h1/d1 and Q holds on h2/d2
                // 3. Asserting domains are disjoint: forall addr. !(d1[addr] && d2[addr])
                // 4. Asserting combined domain equals parent: forall addr. domain[addr] <=> (d1[addr] || d2[addr])
                // 5. Asserting heap merge: forall addr. (d1[addr] => heap[addr] = h1[addr]) &&
                //                                        (d2[addr] => heap[addr] = h2[addr])

                // Generate unique names using a counter to avoid name conflicts in nested separating conjunctions
                static SEP_COUNTER: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(0);
                let counter = SEP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                let heap1 = z3::ast::Array::new_const(
                    format!("_heap1_{}", counter).as_str(),
                    &z3::Sort::int(),
                    &z3::Sort::int(),
                );
                let heap2 = z3::ast::Array::new_const(
                    format!("_heap2_{}", counter).as_str(),
                    &z3::Sort::int(),
                    &z3::Sort::int(),
                );
                let domain1 = z3::ast::Array::new_const(
                    format!("_domain1_{}", counter).as_str(),
                    &z3::Sort::int(),
                    &z3::Sort::bool(),
                );
                let domain2 = z3::ast::Array::new_const(
                    format!("_domain2_{}", counter).as_str(),
                    &z3::Sort::int(),
                    &z3::Sort::bool(),
                );

                // Sub-propositions hold on their respective heaps
                let p_holds = self.encode_sep_prop(p, &heap1, &domain1)?;
                let q_holds = self.encode_sep_prop(q, &heap2, &domain2)?;

                // Create a fresh address variable for quantified assertions
                let addr = z3::ast::Int::new_const(format!("_sep_addr_{}", counter).as_str());

                // Domain lookup for sub-heaps
                let d1_at_addr = domain1
                    .select(&addr)
                    .as_bool()
                    .ok_or_else(|| WPError::TypeError(Text::from("Domain1 should be boolean")))?;
                let d2_at_addr = domain2
                    .select(&addr)
                    .as_bool()
                    .ok_or_else(|| WPError::TypeError(Text::from("Domain2 should be boolean")))?;
                let parent_domain_at_addr = domain.select(&addr).as_bool().ok_or_else(|| {
                    WPError::TypeError(Text::from("Parent domain should be boolean"))
                })?;

                // Constraint 1: Domains are disjoint - forall addr. !(d1[addr] && d2[addr])
                let disjoint_constraint = z3::ast::Bool::and(&[&d1_at_addr, &d2_at_addr]).not();
                let all_disjoint = z3::ast::forall_const(
                    &[&z3::ast::Dynamic::from(addr.clone())],
                    &[],
                    &disjoint_constraint,
                );

                // Constraint 2: Combined domain equals parent - forall addr. parent_domain[addr] <=> (d1[addr] || d2[addr])
                let combined_domain = z3::ast::Bool::or(&[&d1_at_addr, &d2_at_addr]);
                let domain_merge = parent_domain_at_addr.iff(&combined_domain);
                let all_domain_merge = z3::ast::forall_const(
                    &[&z3::ast::Dynamic::from(addr.clone())],
                    &[],
                    &domain_merge,
                );

                // Constraint 3: Heap merge - values from sub-heaps propagate to parent
                // forall addr. (d1[addr] => heap[addr] = h1[addr]) && (d2[addr] => heap[addr] = h2[addr])
                let h1_at_addr = heap1.select(&addr);
                let h2_at_addr = heap2.select(&addr);
                let parent_heap_at_addr = heap.select(&addr);

                let heap1_merge = d1_at_addr.implies(&parent_heap_at_addr.eq(h1_at_addr));
                let heap2_merge = d2_at_addr.implies(&parent_heap_at_addr.eq(h2_at_addr));
                let heap_merge_constraint = z3::ast::Bool::and(&[&heap1_merge, &heap2_merge]);
                let all_heap_merge = z3::ast::forall_const(
                    &[&z3::ast::Dynamic::from(addr)],
                    &[],
                    &heap_merge_constraint,
                );

                // Combine all constraints: P(h1) && Q(h2) && disjoint && domain_merge && heap_merge
                Ok(z3::ast::Bool::and(&[
                    &p_holds,
                    &q_holds,
                    &all_disjoint,
                    &all_domain_merge,
                    &all_heap_merge,
                ]))
            }
            SepProp::Pure(formula) => {
                // Pure formula doesn't constrain heap
                let hoare_verifier = HoareZ3Verifier::new(self.context);
                hoare_verifier.translate_formula(formula, &Map::new())
            }
            SepProp::MagicWand(p, q) => {
                // p -* q: if we add p to current heap, we get q
                // Encode as: forall h'. (current * p)(h') => q(h')
                let p_formula = self.encode_sep_prop(p, heap, domain)?;
                let q_formula = self.encode_sep_prop(q, heap, domain)?;
                Ok(p_formula.implies(&q_formula))
            }
            SepProp::Exists(vars, body) => {
                let mut bounds: Vec<z3::ast::Dynamic> = Vec::new();
                for var in vars.iter() {
                    bounds.push(z3::ast::Int::new_const(var.smtlib_name().as_str()).into());
                }
                let body_formula = self.encode_sep_prop(body, heap, domain)?;
                let bound_refs: Vec<&dyn z3::ast::Ast> =
                    bounds.iter().map(|b| b as &dyn z3::ast::Ast).collect();
                Ok(z3::ast::exists_const(&bound_refs, &[], &body_formula))
            }
            SepProp::Forall(vars, body) => {
                let mut bounds: Vec<z3::ast::Dynamic> = Vec::new();
                for var in vars.iter() {
                    bounds.push(z3::ast::Int::new_const(var.smtlib_name().as_str()).into());
                }
                let body_formula = self.encode_sep_prop(body, heap, domain)?;
                let bound_refs: Vec<&dyn z3::ast::Ast> =
                    bounds.iter().map(|b| b as &dyn z3::ast::Ast).collect();
                Ok(z3::ast::forall_const(&bound_refs, &[], &body_formula))
            }
            SepProp::Predicate(name, args) => {
                // Custom predicate - encode as uninterpreted function
                let hoare_verifier = HoareZ3Verifier::new(self.context);
                let z3_args: Result<Vec<_>, _> = args
                    .iter()
                    .map(|expr| hoare_verifier.translate_expr(expr, &Map::new()))
                    .collect();
                let z3_args = z3_args?;

                let arg_sorts: Vec<_> = args.iter().map(|_| z3::Sort::int()).collect();
                let arg_sort_refs: Vec<_> = arg_sorts.iter().collect();
                let pred_decl = z3::FuncDecl::new(name.as_str(), &arg_sort_refs, &z3::Sort::bool());

                let arg_refs: Vec<_> = z3_args.iter().map(|a| a as &dyn z3::ast::Ast).collect();
                pred_decl.apply(&arg_refs).as_bool().ok_or_else(|| {
                    WPError::TypeError(Text::from("Predicate should return boolean"))
                })
            }
            SepProp::FieldPointsTo(addr, _field, val) => {
                // addr.field |-> val: similar to PointsTo but for specific field
                let z3_addr = self.encode_address(addr)?;
                let z3_val = self.encode_value(val)?;

                let heap_val = heap.select(&z3_addr);
                let domain_val = domain.select(&z3_addr);

                let heap_eq = heap_val.eq(z3::ast::Dynamic::from(z3_val));
                let in_domain = domain_val
                    .as_bool()
                    .ok_or_else(|| WPError::TypeError(Text::from("Domain should be boolean")))?;

                Ok(z3::ast::Bool::and(&[&heap_eq, &in_domain]))
            }
        }
    }

    /// Encode heap command semantics
    fn encode_command(
        &self,
        cmd: &HeapCommand,
        heap: &z3::ast::Array,
        domain: &z3::ast::Array,
    ) -> Result<(z3::ast::Array, z3::ast::Array), WPError> {
        match cmd {
            HeapCommand::Alloc(var, val) => {
                // Allocate: choose fresh address, store value
                let addr = z3::ast::Int::new_const(var.smtlib_name().as_str());
                let z3_val = self.encode_value(val)?;

                let new_heap = heap.store(&addr, &z3_val);
                let new_domain = domain.store(&addr, &z3::ast::Bool::from_bool(true));

                Ok((new_heap, new_domain))
            }
            HeapCommand::Free(addr) => {
                // Free: remove from domain
                let z3_addr = self.encode_address(addr)?;
                let new_domain = domain.store(&z3_addr, &z3::ast::Bool::from_bool(false));
                Ok((heap.clone(), new_domain))
            }
            HeapCommand::Load(var, addr) => {
                // Load doesn't modify heap
                Ok((heap.clone(), domain.clone()))
            }
            HeapCommand::Store(addr, val) => {
                // Store: update heap value
                let z3_addr = self.encode_address(addr)?;
                let z3_val = self.encode_value(val)?;
                let new_heap = heap.store(&z3_addr, &z3_val);
                Ok((new_heap, domain.clone()))
            }
        }
    }

    /// Encode address to Z3
    fn encode_address(&self, addr: &Address) -> Result<z3::ast::Int, WPError> {
        let hoare_verifier = HoareZ3Verifier::new(self.context);
        hoare_verifier.translate_expr_as_int(&addr.0, &Map::new())
    }

    /// Encode value to Z3
    fn encode_value(&self, val: &Value) -> Result<z3::ast::Int, WPError> {
        match val {
            Value::Int(expr) => {
                let hoare_verifier = HoareZ3Verifier::new(self.context);
                hoare_verifier.translate_expr_as_int(expr, &Map::new())
            }
            Value::Bool(expr) => {
                // Encode boolean as int expression
                let hoare_verifier = HoareZ3Verifier::new(self.context);
                hoare_verifier.translate_expr_as_int(expr, &Map::new())
            }
            Value::Symbolic(var) => Ok(z3::ast::Int::new_const(var.smtlib_name().as_str())),
            Value::Addr(addr) => self.encode_address(addr),
            Value::Struct(_, values) => {
                // For now, encode first field or 0
                if let Some(first) = values.first() {
                    self.encode_value(first)
                } else {
                    Ok(z3::ast::Int::from_i64(0))
                }
            }
        }
    }

    /// Extract counterexample from model
    fn extract_heap_counterexample(
        &self,
        model: &z3::Model,
        heap_pre: &z3::ast::Array,
        heap_post: &z3::ast::Array,
    ) -> HeapCounterexample {
        let mut pre_values = Map::new();
        let mut post_values = Map::new();

        // Sample a few addresses to show counterexample
        for i in 0..5 {
            let addr = z3::ast::Int::from_i64(i);
            if let Some(pre_val) = model.eval(&heap_pre.select(&addr), true) {
                pre_values.insert(
                    Text::from(format!("{}", i)),
                    Text::from(format!("{}", pre_val)),
                );
            }
            if let Some(post_val) = model.eval(&heap_post.select(&addr), true) {
                post_values.insert(
                    Text::from(format!("{}", i)),
                    Text::from(format!("{}", post_val)),
                );
            }
        }

        HeapCounterexample {
            pre_heap: pre_values,
            post_heap: post_values,
        }
    }
}

/// Counterexample for separation logic verification
#[derive(Debug, Clone, Default)]
pub struct HeapCounterexample {
    /// Heap values before command
    pub pre_heap: Map<Text, Text>,
    /// Heap values after command
    pub post_heap: Map<Text, Text>,
}

/// Result of separation logic verification
#[derive(Debug, Clone)]
pub struct SepLogicVerificationResult {
    /// Whether the triple is valid
    pub valid: bool,
    /// Counterexample if invalid
    pub counterexample: Option<HeapCounterexample>,
}

impl SepLogicVerificationResult {
    /// Create a valid result
    pub fn valid() -> Self {
        Self {
            valid: true,
            counterexample: None,
        }
    }

    /// Create an invalid result
    pub fn invalid(counterexample: HeapCounterexample) -> Self {
        Self {
            valid: false,
            counterexample: Some(counterexample),
        }
    }
}
