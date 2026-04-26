//! SMT-backed guard verification — moved from
//! `verum_types::exhaustiveness::smt` to break the
//! `verum_types ↔ verum_smt` circular dependency.
//!
//! The public types consumed by callers (`GuardedPattern`,
//! `SmtGuardConfig`, `SmtGuardResult`, `SmtValue`, `SmtWitness`) stay in
//! `verum_types::exhaustiveness` so `verum_types` can express guard
//! analysis as data without linking to Z3. This module implements
//! `GuardVerifier` for the SMT-backed path.

use std::collections::HashMap;
use std::time::Duration;

use verum_ast::expr::Expr;
use verum_common::{List, Text};

use verum_types::context::TypeEnv;
use verum_types::exhaustiveness::diagnostics::{ExhaustivenessWarning};
use verum_types::exhaustiveness::smt::{
    GuardedPattern, GuardVerifier, SmtGuardConfig, SmtGuardResult, SmtValue, SmtWitness,
};
use verum_types::ty::Type;

/// SMT-based guard verifier
///
/// Manages Z3 context and provides methods for analyzing guard conditions
/// using SMT solving. Implements the `GuardVerifier` trait so callers can
/// hold a `&dyn GuardVerifier` without linking Z3.
pub struct SmtGuardVerifier {
    config: SmtGuardConfig,
}

impl SmtGuardVerifier {
    /// Create a new SMT guard verifier
    pub fn new(config: SmtGuardConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(SmtGuardConfig::default())
    }

    /// Check if a type can be represented in SMT
    fn is_smt_compatible(&self, ty: &Type) -> bool {
        matches!(ty, Type::Int | Type::Float | Type::Bool | Type::Char)
    }

    /// Translate guard expressions to SMT formulas
    fn translate_guards(
        &self,
        guards: &[GuardedPattern],
        _scrutinee_ty: &Type,
    ) -> Result<List<SmtFormula>, Vec<usize>> {
        let mut formulas = List::new();
        let mut unknown = Vec::new();

        for (i, guard) in guards.iter().enumerate() {
            match self.translate_expr(&guard.guard, &guard.bound_vars) {
                Some(formula) => formulas.push(formula),
                None => unknown.push(i),
            }
        }

        if unknown.is_empty() {
            Ok(formulas)
        } else {
            Err(unknown)
        }
    }

    /// Translate a single expression to SMT formula
    fn translate_expr(&self, expr: &Expr, bound_vars: &HashMap<Text, Type>) -> Option<SmtFormula> {
        use verum_ast::expr::{BinOp, ExprKind, UnOp};
        use verum_ast::literal::LiteralKind;

        match &expr.kind {
            // Comparison operators
            ExprKind::Binary { op, left, right } => {
                let l = self.translate_expr(left, bound_vars)?;
                let r = self.translate_expr(right, bound_vars)?;

                let smt_op = match op {
                    BinOp::Lt => SmtOp::Lt,
                    BinOp::Le => SmtOp::Le,
                    BinOp::Gt => SmtOp::Gt,
                    BinOp::Ge => SmtOp::Ge,
                    BinOp::Eq => SmtOp::Eq,
                    BinOp::Ne => SmtOp::Ne,
                    BinOp::And => SmtOp::And,
                    BinOp::Or => SmtOp::Or,
                    BinOp::Add => SmtOp::Add,
                    BinOp::Sub => SmtOp::Sub,
                    BinOp::Mul => SmtOp::Mul,
                    BinOp::Div => SmtOp::Div,
                    BinOp::Rem => SmtOp::Mod,
                    _ => return None,
                };

                Some(SmtFormula::Binary {
                    op: smt_op,
                    left: Box::new(l),
                    right: Box::new(r),
                })
            }

            // Unary operators
            ExprKind::Unary { op, expr: inner } => {
                let translated = self.translate_expr(inner, bound_vars)?;

                match op {
                    UnOp::Not => Some(SmtFormula::Not(Box::new(translated))),
                    UnOp::Neg => Some(SmtFormula::Neg(Box::new(translated))),
                    _ => None,
                }
            }

            // Literals
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(int_lit) => Some(SmtFormula::Int(int_lit.value)),
                LiteralKind::Bool(b) => Some(SmtFormula::Bool(*b)),
                _ => None,
            },

            // Variable references (via Path)
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                        let name = Text::from(ident.name.as_str());
                        if bound_vars.contains_key(&name) {
                            return Some(SmtFormula::Var(name));
                        }
                    }
                }
                None
            }

            // Parentheses
            ExprKind::Paren(inner) => self.translate_expr(inner, bound_vars),

            _ => None,
        }
    }

    /// Check if all guards together are exhaustive
    fn check_exhaustiveness(&self, formulas: &[SmtFormula], scrutinee_ty: &Type) -> bool {
        if formulas.is_empty() {
            return false;
        }

        let disjunction = if formulas.len() == 1 {
            formulas[0].clone()
        } else {
            SmtFormula::Or(formulas.iter().cloned().collect())
        };

        let negation = SmtFormula::Not(Box::new(disjunction));

        self.check_unsat(&negation, scrutinee_ty)
    }

    /// Find redundant guards (guards that are subsumed by earlier ones)
    fn find_redundant_guards(&self, formulas: &[SmtFormula]) -> List<usize> {
        let mut redundant = List::new();

        for i in 1..formulas.len() {
            let earlier: List<SmtFormula> = formulas[..i].iter().cloned().collect();
            let earlier_disjunction = if earlier.len() == 1 {
                earlier[0].clone()
            } else {
                SmtFormula::Or(earlier)
            };

            let check = SmtFormula::Binary {
                op: SmtOp::And,
                left: Box::new(formulas[i].clone()),
                right: Box::new(SmtFormula::Not(Box::new(earlier_disjunction))),
            };

            if self.check_unsat(&check, &Type::Bool) {
                redundant.push(i);
            }
        }

        redundant
    }

    /// Extract witness values for uncovered cases
    fn extract_uncovered_witnesses(
        &self,
        formulas: &[SmtFormula],
        _scrutinee_ty: &Type,
    ) -> List<SmtWitness> {
        if formulas.is_empty() {
            return List::new();
        }

        let disjunction = if formulas.len() == 1 {
            formulas[0].clone()
        } else {
            SmtFormula::Or(formulas.iter().cloned().collect())
        };

        let negation = SmtFormula::Not(Box::new(disjunction));

        if let Some(model) = self.get_model(&negation) {
            List::from_iter([model])
        } else {
            List::new()
        }
    }

    /// Check if a formula is unsatisfiable using Z3
    fn check_unsat(&self, formula: &SmtFormula, scrutinee_ty: &Type) -> bool {
        use crate::z3_backend::{Z3Config, Z3ContextManager};

        let mut config = Z3Config::default();
        config.global_timeout_ms = verum_common::Maybe::Some(self.config.timeout_ms);
        config.enable_proofs = false;

        let ctx_manager = Z3ContextManager::new(config);

        ctx_manager.with_config(|| {
            use z3::Solver;
            use z3::SatResult;

            let solver = Solver::new();

            if let Some(z3_formula) = self.formula_to_z3(formula, scrutinee_ty) {
                solver.assert(&z3_formula);
                matches!(solver.check(), SatResult::Unsat)
            } else {
                false
            }
        })
    }

    /// Translate our SMT formula to Z3 AST
    fn formula_to_z3(&self, formula: &SmtFormula, scrutinee_ty: &Type) -> Option<z3::ast::Bool> {
        use z3::ast::Bool;

        match formula {
            SmtFormula::Bool(b) => Some(Bool::from_bool(*b)),
            SmtFormula::Int(_) => None,
            SmtFormula::Var(name) => match scrutinee_ty {
                Type::Int => None,
                Type::Bool => Some(Bool::new_const(name.as_str())),
                _ => None,
            },
            SmtFormula::Binary { op, left, right } => {
                self.translate_binary(*op, left, right, scrutinee_ty)
            }
            SmtFormula::Not(inner) => {
                let inner_z3 = self.formula_to_z3(inner, scrutinee_ty)?;
                Some(inner_z3.not())
            }
            SmtFormula::Or(formulas) => {
                let z3_formulas: Option<Vec<_>> = formulas
                    .iter()
                    .map(|f| self.formula_to_z3(f, scrutinee_ty))
                    .collect();
                let z3_formulas = z3_formulas?;
                let refs: Vec<_> = z3_formulas.iter().collect();
                Some(Bool::or(&refs))
            }
            SmtFormula::Neg(_) => None,
        }
    }

    /// Translate a binary operation to Z3
    fn translate_binary(
        &self,
        op: SmtOp,
        left: &SmtFormula,
        right: &SmtFormula,
        scrutinee_ty: &Type,
    ) -> Option<z3::ast::Bool> {
        use z3::ast::{Bool, Int};

        fn to_int(f: &SmtFormula) -> Option<Int> {
            match f {
                SmtFormula::Int(n) => Some(Int::from_i64(*n as i64)),
                SmtFormula::Var(name) => Some(Int::new_const(name.as_str())),
                SmtFormula::Binary { op, left, right } => {
                    let l = to_int(left)?;
                    let r = to_int(right)?;
                    match op {
                        SmtOp::Add => Some(l + r),
                        SmtOp::Sub => Some(l - r),
                        SmtOp::Mul => Some(l * r),
                        SmtOp::Div => Some(l / r),
                        SmtOp::Mod => Some(l % r),
                        _ => None,
                    }
                }
                SmtFormula::Neg(inner) => {
                    let i = to_int(inner)?;
                    Some(-i)
                }
                _ => None,
            }
        }

        match op {
            SmtOp::Lt => {
                let l = to_int(left)?;
                let r = to_int(right)?;
                Some(l.lt(&r))
            }
            SmtOp::Le => {
                let l = to_int(left)?;
                let r = to_int(right)?;
                Some(l.le(&r))
            }
            SmtOp::Gt => {
                let l = to_int(left)?;
                let r = to_int(right)?;
                Some(l.gt(&r))
            }
            SmtOp::Ge => {
                let l = to_int(left)?;
                let r = to_int(right)?;
                Some(l.ge(&r))
            }
            SmtOp::Eq => {
                let l = to_int(left)?;
                let r = to_int(right)?;
                Some(l.eq(&r))
            }
            SmtOp::Ne => {
                let l = to_int(left)?;
                let r = to_int(right)?;
                Some(l.eq(&r).not())
            }
            SmtOp::And => {
                let l = self.formula_to_z3(left, scrutinee_ty)?;
                let r = self.formula_to_z3(right, scrutinee_ty)?;
                Some(Bool::and(&[&l, &r]))
            }
            SmtOp::Or => {
                let l = self.formula_to_z3(left, scrutinee_ty)?;
                let r = self.formula_to_z3(right, scrutinee_ty)?;
                Some(Bool::or(&[&l, &r]))
            }
            SmtOp::Add | SmtOp::Sub | SmtOp::Mul | SmtOp::Div | SmtOp::Mod => None,
        }
    }

    /// Get a satisfying model for a formula
    fn get_model(&self, formula: &SmtFormula) -> Option<SmtWitness> {
        use crate::z3_backend::{Z3Config, Z3ContextManager};

        let mut config = Z3Config::default();
        config.global_timeout_ms = verum_common::Maybe::Some(self.config.timeout_ms);
        config.enable_proofs = false;

        let ctx_manager = Z3ContextManager::new(config);

        ctx_manager.with_config(|| {
            use z3::Solver;
            use z3::SatResult;
            use z3::ast::Int;

            let solver = Solver::new();

            if let Some(z3_formula) = self.formula_to_z3(formula, &Type::Int) {
                solver.assert(&z3_formula);

                if let SatResult::Sat = solver.check() {
                    if let Some(model) = solver.get_model() {
                        let mut bindings = HashMap::new();

                        let n_var = Int::new_const("n");
                        if let Some(value) = model.eval(&n_var, true) {
                            if let Some(int_val) = value.as_i64() {
                                bindings.insert(Text::from("n"), SmtValue::Int(int_val as i128));
                            }
                        }

                        return Some(SmtWitness {
                            bindings,
                            description: Text::from("Uncovered case found by SMT"),
                        });
                    }
                }
            }
            None
        })
    }

    /// Verify guards (primary entry point, also surfaced via `GuardVerifier` trait).
    pub fn verify_guards(
        &self,
        guards: &[GuardedPattern],
        scrutinee_ty: &Type,
        _env: &TypeEnv,
    ) -> SmtGuardResult {
        if guards.is_empty() {
            return SmtGuardResult::empty();
        }

        if guards.len() > self.config.max_guards {
            return SmtGuardResult::skipped(format!(
                "too many guards ({} > {})",
                guards.len(),
                self.config.max_guards
            ));
        }

        if !self.is_smt_compatible(scrutinee_ty) {
            return SmtGuardResult::skipped("scrutinee type not SMT-compatible");
        }

        let start = std::time::Instant::now();

        let formulas = match self.translate_guards(guards, scrutinee_ty) {
            Ok(f) => f,
            Err(indices) => {
                return SmtGuardResult {
                    is_exhaustive: false,
                    redundant_guards: List::new(),
                    uncovered_witnesses: List::new(),
                    unknown_guards: List::from_iter(indices),
                    solve_time: start.elapsed(),
                    skipped: false,
                    skip_reason: None,
                };
            }
        };

        let is_exhaustive = self.check_exhaustiveness(&formulas, scrutinee_ty);

        let redundant = if self.config.detect_redundancy {
            self.find_redundant_guards(&formulas)
        } else {
            List::new()
        };

        let witnesses = if !is_exhaustive && self.config.extract_witnesses {
            self.extract_uncovered_witnesses(&formulas, scrutinee_ty)
        } else {
            List::new()
        };

        SmtGuardResult {
            is_exhaustive,
            redundant_guards: redundant,
            uncovered_witnesses: witnesses,
            unknown_guards: List::new(),
            solve_time: start.elapsed(),
            skipped: false,
            skip_reason: None,
        }
    }
}

impl GuardVerifier for SmtGuardVerifier {
    fn verify_guards(
        &self,
        patterns: &[GuardedPattern],
        scrutinee_ty: &Type,
        env: &TypeEnv,
    ) -> SmtGuardResult {
        Self::verify_guards(self, patterns, scrutinee_ty, env)
    }
}

/// Internal SMT formula representation (private — this module owns the Z3
/// lowering).
#[derive(Debug, Clone)]
enum SmtFormula {
    Bool(bool),
    Int(i128),
    Var(Text),
    Binary {
        op: SmtOp,
        left: Box<SmtFormula>,
        right: Box<SmtFormula>,
    },
    Not(Box<SmtFormula>),
    Neg(Box<SmtFormula>),
    Or(List<SmtFormula>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmtOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// Suppress unused-field warning — field is kept for error reporting symmetry.
const _: Duration = Duration::ZERO;

/// Analyze guards in a match and produce warnings if appropriate.
///
/// Moved from `verum_types::exhaustiveness::smt::analyze_guarded_match` so
/// callers that already depend on `verum_smt` can use the SMT-backed
/// analysis directly.
pub fn analyze_guarded_match(
    guards: &[GuardedPattern],
    scrutinee_ty: &Type,
    _env: &TypeEnv,
    span: Option<verum_ast::span::Span>,
) -> List<ExhaustivenessWarning> {
    let mut warnings = List::new();

    if !guards.is_empty() {
        let verifier = SmtGuardVerifier::with_defaults();
        let result = verifier.verify_guards(guards, scrutinee_ty, _env);

        if !result.is_exhaustive && !result.skipped {
            warnings.push(ExhaustivenessWarning::all_guarded(span));
        }

        for idx in result.redundant_guards.iter() {
            warnings.push(ExhaustivenessWarning::unreachable(*idx, span));
        }
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_smt_compatible() {
        let verifier = SmtGuardVerifier::with_defaults();
        assert!(verifier.is_smt_compatible(&Type::Int));
        assert!(verifier.is_smt_compatible(&Type::Bool));
        assert!(verifier.is_smt_compatible(&Type::Float));
        assert!(!verifier.is_smt_compatible(&Type::Text));
        assert!(!verifier.is_smt_compatible(&Type::Unit));
    }

    #[test]
    fn test_empty_guards() {
        let verifier = SmtGuardVerifier::with_defaults();
        let env = TypeEnv::new();
        let result = verifier.verify_guards(&[], &Type::Int, &env);
        assert!(!result.is_exhaustive);
        assert!(!result.skipped);
    }
}
