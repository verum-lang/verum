//! SMT-Backed Guard Verification
//!
//! This module provides Z3-based verification for complex guard conditions
//! in pattern matching. When guards contain arithmetic expressions, comparisons,
//! or logical operations, we can use SMT solving to determine:
//!
//! 1. Whether guards are exhaustive (cover all remaining cases)
//! 2. Whether guards are redundant (overlap with earlier patterns)
//! 3. Concrete witness values when guards leave gaps
//!
//! ## Integration with Exhaustiveness
//!
//! The main exhaustiveness checker treats guards conservatively (as potentially failing).
//! This module provides precise analysis for cases where:
//! - A match has only guarded arms (E0603 warning candidate)
//! - Guards use arithmetic that can be proven exhaustive
//! - Guards are demonstrably redundant via SMT
//!
//! ## Example
//!
//! ```verum
//! match x {
//!     n if n < 0 => negative(),
//!     n if n == 0 => zero(),
//!     n if n > 0 => positive(),  // SMT proves: exhaustive!
//! }
//! ```

use super::diagnostics::{ExhaustivenessWarning, ExhaustivenessWarningCode};
use super::matrix::PatternColumn;
use crate::context::TypeEnv;
use crate::ty::Type;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use verum_ast::expr::Expr;
use verum_common::{List, Maybe, Text};

/// Configuration for SMT-backed guard verification
#[derive(Debug, Clone)]
pub struct SmtGuardConfig {
    /// Timeout for individual guard checks (default: 100ms)
    pub timeout_ms: u64,
    /// Maximum number of guards to analyze with SMT (default: 10)
    pub max_guards: usize,
    /// Enable witness extraction for uncovered cases
    pub extract_witnesses: bool,
    /// Enable guard redundancy detection
    pub detect_redundancy: bool,
}

impl Default for SmtGuardConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 100,
            max_guards: 10,
            extract_witnesses: true,
            detect_redundancy: true,
        }
    }
}

/// Result of SMT guard verification
#[derive(Debug, Clone)]
pub struct SmtGuardResult {
    /// Whether all guards together are provably exhaustive
    pub is_exhaustive: bool,
    /// Indices of redundant guards (covered by earlier guards)
    pub redundant_guards: List<usize>,
    /// Witness values for uncovered cases (if any)
    pub uncovered_witnesses: List<SmtWitness>,
    /// Guards that couldn't be analyzed (too complex for SMT)
    pub unknown_guards: List<usize>,
    /// Time spent in SMT solving
    pub solve_time: Duration,
    /// Whether SMT analysis was skipped (too many guards, etc.)
    pub skipped: bool,
    /// Reason for skipping, if applicable
    pub skip_reason: Option<Text>,
}

impl SmtGuardResult {
    /// Create a result for when SMT analysis is skipped
    pub fn skipped(reason: impl Into<Text>) -> Self {
        Self {
            is_exhaustive: false,
            redundant_guards: List::new(),
            uncovered_witnesses: List::new(),
            unknown_guards: List::new(),
            solve_time: Duration::ZERO,
            skipped: true,
            skip_reason: Some(reason.into()),
        }
    }

    /// Create an empty result
    pub fn empty() -> Self {
        Self {
            is_exhaustive: false,
            redundant_guards: List::new(),
            uncovered_witnesses: List::new(),
            unknown_guards: List::new(),
            solve_time: Duration::ZERO,
            skipped: false,
            skip_reason: None,
        }
    }
}

/// A witness value extracted from SMT model
#[derive(Debug, Clone)]
pub struct SmtWitness {
    /// Variable name -> value mapping
    pub bindings: HashMap<Text, SmtValue>,
    /// Human-readable description
    pub description: Text,
}

/// Concrete value from SMT model
#[derive(Debug, Clone)]
pub enum SmtValue {
    Int(i128),
    Float(f64),
    Bool(bool),
    Unknown,
}

impl std::fmt::Display for SmtValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmtValue::Int(n) => write!(f, "{}", n),
            SmtValue::Float(n) => write!(f, "{}", n),
            SmtValue::Bool(b) => write!(f, "{}", b),
            SmtValue::Unknown => write!(f, "_"),
        }
    }
}

/// Guard expression with its pattern context
#[derive(Debug, Clone)]
pub struct GuardedPattern {
    /// Index in the original pattern list
    pub pattern_index: usize,
    /// The base pattern (without guard)
    pub base_pattern: PatternColumn,
    /// The guard expression
    pub guard: Arc<Expr>,
    /// Variables bound by the pattern
    pub bound_vars: HashMap<Text, Type>,
}

/// SMT-based guard verifier
///
/// This struct manages Z3 context and provides methods for analyzing
/// guard conditions using SMT solving.
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

    /// Verify whether guarded patterns are exhaustive for a type
    ///
    /// This analyzes the guard expressions using SMT to determine:
    /// 1. Whether all guards together cover all possible values
    /// 2. Which guards are redundant
    /// 3. Example values for uncovered cases
    pub fn verify_guards(
        &self,
        guards: &[GuardedPattern],
        scrutinee_ty: &Type,
        env: &TypeEnv,
    ) -> SmtGuardResult {
        // Check if we should skip SMT analysis
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

        // Check if scrutinee type is SMT-compatible
        if !self.is_smt_compatible(scrutinee_ty) {
            return SmtGuardResult::skipped("scrutinee type not SMT-compatible");
        }

        let start = std::time::Instant::now();

        // Translate guards to SMT formulas
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

        // Check exhaustiveness: Is there any value that doesn't satisfy any guard?
        let is_exhaustive = self.check_exhaustiveness(&formulas, scrutinee_ty);

        // Check redundancy: Does each guard add new coverage?
        let redundant = if self.config.detect_redundancy {
            self.find_redundant_guards(&formulas)
        } else {
            List::new()
        };

        // Extract witnesses for uncovered cases
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

    /// Check if a type can be represented in SMT
    fn is_smt_compatible(&self, ty: &Type) -> bool {
        matches!(ty, Type::Int | Type::Float | Type::Bool | Type::Char)
    }

    /// Translate guard expressions to SMT formulas
    ///
    /// Returns Ok with formulas if all guards can be translated,
    /// or Err with indices of untranslatable guards.
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
            ExprKind::Literal(lit) => {
                match &lit.kind {
                    LiteralKind::Int(int_lit) => Some(SmtFormula::Int(int_lit.value)),
                    LiteralKind::Float(float_lit) => Some(SmtFormula::Float(float_lit.value)),
                    LiteralKind::Bool(b) => Some(SmtFormula::Bool(*b)),
                    _ => None,
                }
            }

            // Variable references (via Path)
            ExprKind::Path(path) => {
                // Extract simple identifier from path
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
        // Build the negation of "some guard is true"
        // If UNSAT, then at least one guard is always true
        if formulas.is_empty() {
            return false;
        }

        // For each possible value, at least one guard should be true
        // We check: NOT(OR(guard1, guard2, ...)) is UNSAT
        let disjunction = if formulas.len() == 1 {
            formulas[0].clone()
        } else {
            SmtFormula::Or(formulas.iter().cloned().collect())
        };

        let negation = SmtFormula::Not(Box::new(disjunction));

        // Use Z3 to check if negation is satisfiable
        self.check_unsat(&negation, scrutinee_ty)
    }

    /// Find redundant guards (guards that are subsumed by earlier ones)
    fn find_redundant_guards(&self, formulas: &[SmtFormula]) -> List<usize> {
        let mut redundant = List::new();

        for i in 1..formulas.len() {
            // Check if guard[i] is subsumed by OR(guard[0], ..., guard[i-1])
            let earlier: List<SmtFormula> = formulas[..i].iter().cloned().collect();
            let earlier_disjunction = if earlier.len() == 1 {
                earlier[0].clone()
            } else {
                SmtFormula::Or(earlier)
            };

            // guard[i] AND NOT(earlier_disjunction) should be UNSAT if redundant
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
        // Build NOT(OR(all guards))
        if formulas.is_empty() {
            return List::new();
        }

        let disjunction = if formulas.len() == 1 {
            formulas[0].clone()
        } else {
            SmtFormula::Or(formulas.iter().cloned().collect())
        };

        let negation = SmtFormula::Not(Box::new(disjunction));

        // Try to get a model for the negation
        if let Some(model) = self.get_model(&negation) {
            List::from_iter([model])
        } else {
            List::new()
        }
    }

    /// Check if a formula is unsatisfiable using Z3
    fn check_unsat(&self, formula: &SmtFormula, scrutinee_ty: &Type) -> bool {
        use verum_smt::z3_backend::{Z3Config, Z3ContextManager};

        // Create Z3 context with timeout configuration
        let mut config = Z3Config::default();
        config.global_timeout_ms = verum_common::Maybe::Some(self.config.timeout_ms);
        config.enable_proofs = false; // Not needed for satisfiability check

        let ctx_manager = Z3ContextManager::new(config);

        // Execute Z3 solving with configured context (Z3 uses thread-local context)
        ctx_manager.with_config(|| {
            use z3::{Solver, SatResult};
            use z3::ast::{Ast, Bool, Int};

            let solver = Solver::new();

            // Translate formula to Z3 AST
            if let Some(z3_formula) = self.formula_to_z3(formula, scrutinee_ty) {
                solver.assert(&z3_formula);
                matches!(solver.check(), SatResult::Unsat)
            } else {
                // Couldn't translate - return conservative result
                false
            }
        })
    }

    /// Translate our SMT formula to Z3 AST
    ///
    /// This function converts our internal SMT formula representation to Z3's
    /// AST format for satisfiability checking. The Z3 context is thread-local
    /// so we don't need to pass it explicitly.
    fn formula_to_z3(
        &self,
        formula: &SmtFormula,
        scrutinee_ty: &Type,
    ) -> Option<z3::ast::Bool> {
        use z3::ast::{Ast, Bool, Int};

        match formula {
            SmtFormula::Bool(b) => Some(Bool::from_bool(*b)),
            SmtFormula::Int(_) => {
                // Can't directly use int as bool, this shouldn't happen in well-formed formulas
                None
            }
            SmtFormula::Var(name) => {
                // Create a Z3 variable based on scrutinee type
                match scrutinee_ty {
                    Type::Int => {
                        // For int variables used in comparisons, create an Int const
                        // The actual bool conversion happens in Binary
                        None // Variables alone aren't booleans
                    }
                    Type::Bool => {
                        Some(Bool::new_const(name.as_str()))
                    }
                    _ => None,
                }
            }
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
            SmtFormula::And(formulas) => {
                let z3_formulas: Option<Vec<_>> = formulas
                    .iter()
                    .map(|f| self.formula_to_z3(f, scrutinee_ty))
                    .collect();
                let z3_formulas = z3_formulas?;
                let refs: Vec<_> = z3_formulas.iter().collect();
                Some(Bool::and(&refs))
            }
            SmtFormula::Neg(_) | SmtFormula::Float(_) => None,
        }
    }

    /// Translate a binary operation to Z3
    ///
    /// Handles comparison operators (Lt, Le, Gt, Ge, Eq, Ne) which produce Bool,
    /// and logical operators (And, Or) which combine Bool operands.
    fn translate_binary(
        &self,
        op: SmtOp,
        left: &SmtFormula,
        right: &SmtFormula,
        scrutinee_ty: &Type,
    ) -> Option<z3::ast::Bool> {
        use z3::ast::{Ast, Bool, Int};

        // Helper to convert formula to Int (using Box to enable recursion in closure)
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
            // Comparison operators - need Int operands, produce Bool
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
            // Logical operators - need Bool operands
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
            // Arithmetic operators shouldn't appear at bool level
            SmtOp::Add | SmtOp::Sub | SmtOp::Mul | SmtOp::Div | SmtOp::Mod => None,
        }
    }

    /// Get a satisfying model for a formula
    ///
    /// Attempts to find concrete values that satisfy the formula, which can be
    /// used as witness examples for uncovered cases.
    fn get_model(&self, formula: &SmtFormula) -> Option<SmtWitness> {
        use verum_smt::z3_backend::{Z3Config, Z3ContextManager};

        let mut config = Z3Config::default();
        config.global_timeout_ms = verum_common::Maybe::Some(self.config.timeout_ms);
        config.enable_proofs = false;

        let ctx_manager = Z3ContextManager::new(config);

        ctx_manager.with_config(|| {
            use z3::{Solver, SatResult};
            use z3::ast::{Ast, Int};

            let solver = Solver::new();

            // Translate formula assuming Int scrutinee (most common case for guards)
            if let Some(z3_formula) = self.formula_to_z3(formula, &Type::Int) {
                solver.assert(&z3_formula);

                if let SatResult::Sat = solver.check() {
                    if let Some(model) = solver.get_model() {
                        // Extract variable values from the model
                        let mut bindings = HashMap::new();

                        // Try to find the main scrutinee variable
                        // For guards, we typically have a single bound variable
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
}

/// Internal SMT formula representation
#[derive(Debug, Clone)]
pub enum SmtFormula {
    /// Boolean constant
    Bool(bool),
    /// Integer constant
    Int(i128),
    /// Float constant
    Float(f64),
    /// Variable reference
    Var(Text),
    /// Binary operation
    Binary {
        op: SmtOp,
        left: Box<SmtFormula>,
        right: Box<SmtFormula>,
    },
    /// Negation
    Not(Box<SmtFormula>),
    /// Numeric negation
    Neg(Box<SmtFormula>),
    /// Disjunction (any of)
    Or(List<SmtFormula>),
    /// Conjunction (all of)
    And(List<SmtFormula>),
}

/// SMT operation kinds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtOp {
    // Comparison
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    // Logical
    And,
    Or,
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// Analyze guards in a match and produce warnings if appropriate
pub fn analyze_guarded_match(
    guards: &[GuardedPattern],
    scrutinee_ty: &Type,
    env: &TypeEnv,
    span: Option<verum_ast::span::Span>,
) -> List<ExhaustivenessWarning> {
    let mut warnings = List::new();

    // If all patterns are guarded, emit W0603 unless SMT proves exhaustive
    if !guards.is_empty() {
        let verifier = SmtGuardVerifier::with_defaults();
        let result = verifier.verify_guards(guards, scrutinee_ty, env);

        if !result.is_exhaustive && !result.skipped {
            warnings.push(ExhaustivenessWarning::all_guarded(span));
        }

        // Add redundancy warnings
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
    fn test_smt_config_default() {
        let config = SmtGuardConfig::default();
        assert_eq!(config.timeout_ms, 100);
        assert_eq!(config.max_guards, 10);
        assert!(config.extract_witnesses);
        assert!(config.detect_redundancy);
    }

    #[test]
    fn test_smt_result_skipped() {
        let result = SmtGuardResult::skipped("test reason");
        assert!(result.skipped);
        assert_eq!(result.skip_reason, Some(Text::from("test reason")));
        assert!(!result.is_exhaustive);
    }

    #[test]
    fn test_smt_value_display() {
        assert_eq!(format!("{}", SmtValue::Int(42)), "42");
        assert_eq!(format!("{}", SmtValue::Bool(true)), "true");
        assert_eq!(format!("{}", SmtValue::Unknown), "_");
    }

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
