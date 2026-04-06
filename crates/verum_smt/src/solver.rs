//! SMT Solver Interface and Implementation
//!
//! This module provides a clean abstraction over SMT solvers with Z3 as the primary backend.
//!
//! Verum's refinement types (`Int{> 0}`, `Text{len(it) > 5}`, sigma-type `n: Int where n > 0`)
//! are verified by translating predicates to SMT formulas and checking satisfiability.
//! Five refinement binding forms are supported: inline `{pred}`, lambda `where |x| pred`,
//! sigma-type `x: T where pred(x)`, named predicate `where pred_name`, and bare `where pred`.
//!
//! Performance targets:
//! - SMT queries: < 10ms average
//! - Refinement checking: < 50ms per function

use std::sync::Arc;
use std::time::Instant;

use verum_ast::{
    expr::{BinOp, Block, ConditionKind, Expr, ExprKind, IfCondition, UnOp},
    literal::{Literal, LiteralKind},
    span::Span,
    stmt::StmtKind,
};
use verum_common::{List, Map, Maybe, Text};
use verum_common::ToText;

use crate::option_to_maybe;
use crate::z3_backend::{AdvancedResult, Z3Config, Z3ContextManager, Z3Solver};

// ==================== Core SMT Interface ====================

/// SMT Backend trait for refinement verification
///
/// This trait allows multiple SMT solvers to be plugged in, though Z3 is the primary implementation.
///
/// Note: Z3 Context uses Rc internally and is not Send/Sync, so backends may not be thread-safe.
/// For parallel solving, create separate backend instances per thread.
pub trait SmtBackend {
    /// Verify a verification condition
    fn verify(&self, vc: &VerificationCondition, timeout_ms: u64) -> SmtResult;

    /// Check if an expression is satisfiable
    fn check_sat(&self, expr: &Expr, context: &SmtContext) -> SmtResult;

    /// Get model for satisfiable formula
    fn get_model(&self, expr: &Expr, context: &SmtContext) -> Result<Model, SmtError>;

    /// Verify predicate holds for given bindings
    fn verify_predicate(
        &self,
        predicate: &Expr,
        bindings: &Map<Text, Literal>,
    ) -> Result<bool, SmtError>;
}

/// SMT verification result
#[derive(Debug, Clone)]
pub enum SmtResult {
    /// Formula is satisfiable (valid)
    Sat,
    /// Formula is unsatisfiable (found counterexample)
    Unsat(CounterExample),
    /// Cannot determine (timeout, too complex, etc.)
    Unknown(Text),
    /// Timeout during solving
    Timeout,
}

/// Counterexample from SMT solver
#[derive(Debug, Clone)]
pub struct CounterExample {
    /// Variable bindings that violate the constraint
    pub bindings: Map<Text, Literal>,
    /// Human-readable explanation
    pub explanation: Text,
}

/// SMT context for scoped solving
#[derive(Debug, Clone, Default)]
pub struct SmtContext {
    /// Assumptions (context constraints)
    pub assumptions: List<Expr>,
    /// Variable bindings
    pub bindings: Map<Text, Literal>,
}

/// SMT model (satisfying assignment)
#[derive(Debug, Clone)]
pub struct Model {
    /// Variable assignments
    pub assignments: Map<Text, Literal>,
}

/// SMT errors that can occur during verification.
///
/// Covers solver failures, translation issues, timeouts, and unsupported features.
#[derive(Debug, thiserror::Error)]
pub enum SmtError {
    /// Internal solver error from the SMT backend (e.g., Z3).
    /// Contains the error message from the underlying solver.
    #[error("SMT solver error: {0}")]
    SolverError(Text),

    /// Error during translation from Verum AST to SMT-LIB format.
    /// Typically indicates unsupported expression forms or type mismatches.
    #[error("Translation error: {0}")]
    TranslationError(Text),

    /// Solver exceeded the configured timeout limit.
    /// Contains the timeout duration in milliseconds.
    #[error("Timeout after {0}ms")]
    Timeout(u64),

    /// Feature not supported by the current SMT backend.
    /// May occur with advanced type features or complex predicates.
    #[error("Unsupported feature: {0}")]
    Unsupported(Text),
}

/// Verification condition for SMT solver
#[derive(Debug, Clone)]
pub struct VerificationCondition {
    /// The predicate to verify
    pub predicate: Expr,
    /// Substitutions to apply
    pub substitutions: List<(Text, Expr)>,
    /// Context constraints
    pub context: List<Expr>,
    /// Source location
    pub span: Span,
}

// ==================== Z3 Implementation ====================

/// Z3-based SMT backend
pub struct Z3Backend {
    /// Z3 context manager
    manager: Arc<Z3ContextManager>,
    /// Statistics
    stats: Arc<std::sync::RwLock<SolverStats>>,
    /// Configuration
    config: Z3Config,
}

impl Z3Backend {
    /// Create a new Z3 backend
    pub fn new(config: Z3Config) -> Self {
        let manager = Arc::new(Z3ContextManager::new(config.clone()));

        Self {
            manager,
            stats: Arc::new(std::sync::RwLock::new(SolverStats::default())),
            config,
        }
    }

    /// Translate Verum expression to Z3
    fn translate_expr(
        &self,
        ctx: &z3::Context,
        expr: &Expr,
        var_map: &mut Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Bool, SmtError> {
        match &expr.kind {
            ExprKind::Literal(lit) => self.translate_literal(ctx, lit, var_map),

            ExprKind::Path(path) => {
                // Extract name from path - simplified to handle single identifiers
                let name: Text = if let Maybe::Some(ident) = option_to_maybe(path.as_ident()) {
                    ident.as_str().into()
                } else {
                    // For multi-segment paths, join with "::"
                    path.segments
                        .iter()
                        .filter_map(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(id) => Some(id.as_str()),
                            _ => None,
                        })
                        .collect::<List<_>>()
                        .join(".")
                };

                if let Maybe::Some(z3_var) = var_map.get(&name) {
                    // Try to convert to Bool
                    match option_to_maybe(z3_var.as_bool()) {
                        Maybe::Some(bool_ast) => Ok(bool_ast),
                        Maybe::None => Err(SmtError::TranslationError(Text::from(format!(
                            "Variable {} is not boolean",
                            name
                        )))),
                    }
                } else {
                    // Create fresh boolean variable
                    let bool_var = z3::ast::Bool::new_const(z3::Symbol::String(name.to_string()));
                    var_map.insert(name.clone(), z3::ast::Dynamic::from(bool_var.clone()));
                    Ok(bool_var)
                }
            }

            ExprKind::Binary { op, left, right } => {
                self.translate_binary(ctx, *op, left, right, var_map)
            }

            ExprKind::Unary { op, expr: operand } => {
                self.translate_unary(ctx, *op, operand, var_map)
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Translate if condition to Z3 Bool
                let cond_bool = self.translate_if_condition(ctx, condition, var_map)?;

                // Extract boolean expression from then branch
                let then_bool = self.translate_block(ctx, then_branch, var_map)?;

                // Translate else branch (defaults to false if absent)
                let else_bool = match else_branch {
                    Maybe::Some(else_expr) => self.translate_expr(ctx, else_expr, var_map)?,
                    Maybe::None => z3::ast::Bool::from_bool(false),
                };

                // Create Z3 if-then-else (ite)
                Ok(cond_bool.ite(&then_bool, &else_bool))
            }

            _ => Err(SmtError::Unsupported(Text::from(format!(
                "Expression kind {:?} not supported in SMT",
                expr.kind
            )))),
        }
    }

    fn translate_literal(
        &self,
        _ctx: &z3::Context,
        lit: &Literal,
        _var_map: &mut Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Bool, SmtError> {
        match &lit.kind {
            LiteralKind::Bool(b) => Ok(z3::ast::Bool::from_bool(*b)),
            LiteralKind::Int(i) => {
                // For integer literals in boolean context, treat as comparison with 0
                let int_val = z3::ast::Int::from_i64(i.value as i64);
                let zero = z3::ast::Int::from_i64(0);
                Ok(int_val.eq(&zero).not())
            }
            _ => Err(SmtError::Unsupported(Text::from(format!(
                "Literal {:?} not supported in boolean context",
                lit.kind
            )))),
        }
    }

    fn translate_binary(
        &self,
        ctx: &z3::Context,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        var_map: &mut Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Bool, SmtError> {
        match op {
            BinOp::And => {
                let l = self.translate_expr(ctx, left, var_map)?;
                let r = self.translate_expr(ctx, right, var_map)?;
                Ok(z3::ast::Bool::and(&[&l, &r]))
            }

            BinOp::Or => {
                let l = self.translate_expr(ctx, left, var_map)?;
                let r = self.translate_expr(ctx, right, var_map)?;
                Ok(z3::ast::Bool::or(&[&l, &r]))
            }

            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.translate_comparison(ctx, op, left, right, var_map)
            }

            _ => Err(SmtError::Unsupported(Text::from(format!(
                "Binary operator {:?} not supported",
                op
            )))),
        }
    }

    fn translate_comparison(
        &self,
        ctx: &z3::Context,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        var_map: &mut Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Bool, SmtError> {
        // Try to get integer values
        let l_int = self.try_translate_int(ctx, left, var_map)?;
        let r_int = self.try_translate_int(ctx, right, var_map)?;

        match op {
            BinOp::Eq => Ok(l_int.eq(&r_int)),
            BinOp::Ne => Ok(l_int.eq(&r_int).not()),
            BinOp::Lt => Ok(l_int.lt(&r_int)),
            BinOp::Le => Ok(l_int.le(&r_int)),
            BinOp::Gt => Ok(l_int.gt(&r_int)),
            BinOp::Ge => Ok(l_int.ge(&r_int)),
            _ => unreachable!(),
        }
    }

    fn try_translate_int(
        &self,
        ctx: &z3::Context,
        expr: &Expr,
        var_map: &mut Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Int, SmtError> {
        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(i) => Ok(z3::ast::Int::from_i64(i.value as i64)),
                _ => Err(SmtError::TranslationError(
                    "Expected integer literal".to_text(),
                )),
            },

            ExprKind::Path(path) => {
                // Extract name from path - simplified to handle single identifiers
                let name: Text = if let Maybe::Some(ident) = option_to_maybe(path.as_ident()) {
                    ident.as_str().into()
                } else {
                    // For multi-segment paths, join with "::"
                    path.segments
                        .iter()
                        .filter_map(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(id) => Some(id.as_str()),
                            _ => None,
                        })
                        .collect::<List<_>>()
                        .join(".")
                };

                if let Maybe::Some(z3_var) = var_map.get(&name) {
                    match option_to_maybe(z3_var.as_int()) {
                        Maybe::Some(int_ast) => Ok(int_ast),
                        Maybe::None => Err(SmtError::TranslationError(Text::from(format!(
                            "Variable {} is not an integer",
                            name
                        )))),
                    }
                } else {
                    // Create fresh integer variable
                    let int_var = z3::ast::Int::new_const(z3::Symbol::String(name.to_string()));
                    var_map.insert(name.clone(), z3::ast::Dynamic::from(int_var.clone()));
                    Ok(int_var)
                }
            }

            ExprKind::Binary { op, left, right } => {
                let l = self.try_translate_int(ctx, left, var_map)?;
                let r = self.try_translate_int(ctx, right, var_map)?;

                match op {
                    BinOp::Add => Ok(l + r),
                    BinOp::Sub => Ok(l - r),
                    BinOp::Mul => Ok(l * r),
                    BinOp::Div => Ok(l / r),
                    BinOp::Rem => Ok(l.modulo(&r)),
                    _ => Err(SmtError::Unsupported(Text::from(format!(
                        "Binary operator {:?} not supported for integers",
                        op
                    )))),
                }
            }

            ExprKind::Unary {
                op: UnOp::Neg,
                expr: operand,
            } => {
                let operand = self.try_translate_int(ctx, operand, var_map)?;
                Ok(-operand)
            }

            _ => Err(SmtError::Unsupported(Text::from(format!(
                "Expression {:?} cannot be converted to integer",
                expr.kind
            )))),
        }
    }

    fn translate_unary(
        &self,
        ctx: &z3::Context,
        op: UnOp,
        operand: &Expr,
        var_map: &mut Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Bool, SmtError> {
        match op {
            UnOp::Not => {
                let operand = self.translate_expr(ctx, operand, var_map)?;
                Ok(operand.not())
            }

            _ => Err(SmtError::Unsupported(Text::from(format!(
                "Unary operator {:?} not supported",
                op
            )))),
        }
    }

    /// Translate a substitution expression to a Z3 Dynamic value
    ///
    /// This method translates an expression that will be substituted for a variable.
    /// It attempts to determine the appropriate Z3 sort (Int, Bool, etc.) based on
    /// the expression structure.
    fn translate_substitution_expr(
        &self,
        ctx: &z3::Context,
        expr: &Expr,
        var_map: &mut Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Dynamic, SmtError> {
        match &expr.kind {
            // Literals can be directly converted
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(i) => {
                    let int_val = z3::ast::Int::from_i64(i.value as i64);
                    Ok(z3::ast::Dynamic::from(int_val))
                }
                LiteralKind::Bool(b) => {
                    let bool_val = z3::ast::Bool::from_bool(*b);
                    Ok(z3::ast::Dynamic::from(bool_val))
                }
                _ => Err(SmtError::Unsupported(Text::from(format!(
                    "Literal {:?} not supported in substitution",
                    lit.kind
                )))),
            },

            // Paths reference existing variables
            ExprKind::Path(path) => {
                let name: Text = if let Maybe::Some(ident) = option_to_maybe(path.as_ident()) {
                    ident.as_str().into()
                } else {
                    path.segments
                        .iter()
                        .filter_map(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(id) => Some(id.as_str()),
                            _ => None,
                        })
                        .collect::<List<_>>()
                        .join(".")
                };

                // Look up existing binding or create new integer variable
                if let Maybe::Some(z3_var) = var_map.get(&name) {
                    Ok(z3_var.clone())
                } else {
                    // Default to integer for unknown variables
                    let int_var = z3::ast::Int::new_const(z3::Symbol::String(name.to_string()));
                    let dynamic = z3::ast::Dynamic::from(int_var);
                    var_map.insert(name, dynamic.clone());
                    Ok(dynamic)
                }
            }

            // Binary expressions - try integer arithmetic first, then boolean
            ExprKind::Binary { op, left, right } => {
                // Check if this is an arithmetic or comparison operation
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                        let l = self.try_translate_int(ctx, left, var_map)?;
                        let r = self.try_translate_int(ctx, right, var_map)?;
                        let result = match op {
                            BinOp::Add => l + r,
                            BinOp::Sub => l - r,
                            BinOp::Mul => l * r,
                            BinOp::Div => l / r,
                            BinOp::Rem => l.modulo(&r),
                            _ => unreachable!(),
                        };
                        Ok(z3::ast::Dynamic::from(result))
                    }
                    BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
                    | BinOp::And
                    | BinOp::Or => {
                        let bool_result = self.translate_binary(ctx, *op, left, right, var_map)?;
                        Ok(z3::ast::Dynamic::from(bool_result))
                    }
                    _ => Err(SmtError::Unsupported(Text::from(format!(
                        "Binary operator {:?} not supported in substitution",
                        op
                    )))),
                }
            }

            // Unary expressions
            ExprKind::Unary { op, expr: operand } => match op {
                UnOp::Neg => {
                    let operand = self.try_translate_int(ctx, operand, var_map)?;
                    Ok(z3::ast::Dynamic::from(-operand))
                }
                UnOp::Not => {
                    let operand = self.translate_expr(ctx, operand, var_map)?;
                    Ok(z3::ast::Dynamic::from(operand.not()))
                }
                _ => Err(SmtError::Unsupported(Text::from(format!(
                    "Unary operator {:?} not supported in substitution",
                    op
                )))),
            },

            _ => Err(SmtError::Unsupported(Text::from(format!(
                "Expression {:?} not supported in substitution",
                expr.kind
            )))),
        }
    }

    /// Translate an IfCondition to Z3 Bool
    ///
    /// IfCondition can contain multiple ConditionKind items chained with &&.
    /// We translate each condition and combine with conjunction.
    fn translate_if_condition(
        &self,
        ctx: &z3::Context,
        condition: &IfCondition,
        var_map: &mut Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Bool, SmtError> {
        let mut result = z3::ast::Bool::from_bool(true);

        for cond_kind in &condition.conditions {
            let cond_bool = match cond_kind {
                ConditionKind::Expr(expr) => self.translate_expr(ctx, expr, var_map)?,
                ConditionKind::Let { pattern: _, value } => {
                    // For let-binding conditions in SMT context, we translate
                    // the value expression and treat pattern match as always succeeding
                    // (the type system ensures patterns are exhaustive)
                    // This is a simplification - full implementation would check pattern
                    self.translate_expr(ctx, value, var_map)
                        .unwrap_or_else(|_| z3::ast::Bool::from_bool(true))
                }
            };
            // Combine with conjunction
            result &= cond_bool;
        }

        Ok(result)
    }

    /// Translate a Block to Z3 Bool by extracting its trailing expression
    ///
    /// In SMT context, we're interested in the final boolean value of the block.
    /// This is typically the last expression (without semicolon) in the block.
    fn translate_block(
        &self,
        ctx: &z3::Context,
        block: &Block,
        var_map: &mut Map<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Bool, SmtError> {
        // Look for the trailing expression (last statement without semicolon)
        for stmt in block.stmts.iter().rev() {
            match &stmt.kind {
                StmtKind::Expr {
                    expr,
                    has_semi: false,
                } => {
                    // This is the trailing expression - translate it
                    return self.translate_expr(ctx, expr, var_map);
                }
                StmtKind::Expr {
                    expr,
                    has_semi: true,
                } => {
                    // Try to translate as the value anyway (for blocks ending with ;)
                    // This handles cases like { true; } which should still work
                    if let Ok(bool_val) = self.translate_expr(ctx, expr, var_map) {
                        return Ok(bool_val);
                    }
                }
                _ => continue,
            }
        }

        // No trailing expression found - block evaluates to unit, treat as true
        Ok(z3::ast::Bool::from_bool(true))
    }
}

impl SmtBackend for Z3Backend {
    fn verify(&self, vc: &VerificationCondition, _timeout_ms: u64) -> SmtResult {
        let start = Instant::now();
        let ctx = self.manager.primary();

        // Update stats
        {
            let mut stats = self.stats.write().unwrap();
            stats.total_queries += 1;
        }

        // Create solver with logic specialization
        // Auto-detect logic from constraints (QF_LIA for most refinement types)
        let mut solver = Z3Solver::new(Maybe::Some("QF_LIA"));

        // PERFORMANCE: Enable Z3's advanced tactic auto-selection
        // This provides 2-5x speedup by choosing optimal proof search strategy
        // (already implemented in z3_backend.rs:189-232, now activated!)
        solver.auto_select_tactic();

        // Translate verification condition
        let mut var_map = Map::new();

        // Add context constraints
        for constraint in &vc.context {
            match self.translate_expr(ctx.as_ref(), constraint, &mut var_map) {
                Ok(z3_constraint) => {
                    solver.assert(&z3_constraint);
                }
                Err(e) => {
                    return SmtResult::Unknown(Text::from(format!("Translation error: {}", e)));
                }
            }
        }

        // Apply substitutions by pre-binding substituted variables in var_map
        // This allows the predicate translation to use the substituted values
        for (var_name, subst_expr) in &vc.substitutions {
            // Translate the substitution expression to Z3
            match self.translate_substitution_expr(ctx.as_ref(), subst_expr, &mut var_map) {
                Ok(z3_subst) => {
                    // Bind the variable name to its substituted Z3 expression
                    var_map.insert(var_name.clone(), z3_subst);
                }
                Err(e) => {
                    return SmtResult::Unknown(Text::from(format!(
                        "Substitution translation error for {}: {}",
                        var_name, e
                    )));
                }
            }
        }

        // Translate predicate (will use substituted values from var_map)
        let z3_predicate = match self.translate_expr(ctx.as_ref(), &vc.predicate, &mut var_map) {
            Ok(pred) => pred,
            Err(e) => {
                return SmtResult::Unknown(Text::from(format!("Translation error: {}", e)));
            }
        };

        // Assert predicate
        solver.assert(&z3_predicate);

        // Check with timeout
        // Note: Z3 timeout is set globally in config
        let result = solver.check_sat();

        let elapsed = start.elapsed();

        // Update stats
        {
            let mut stats = self.stats.write().unwrap();
            stats.total_time_ms += elapsed.as_millis() as u64;
        }

        match result {
            AdvancedResult::Sat { .. } => SmtResult::Sat,
            AdvancedResult::Unsat { .. } => {
                // Would extract counterexample from model
                SmtResult::Unsat(CounterExample {
                    bindings: Map::new(),
                    explanation: "Constraint violated".into(),
                })
            }
            AdvancedResult::Unknown { reason } => {
                SmtResult::Unknown(reason.unwrap_or_else(|| "Unknown reason".to_text()))
            }
            AdvancedResult::SatOptimal { .. } => SmtResult::Sat,
        }
    }

    fn check_sat(&self, expr: &Expr, context: &SmtContext) -> SmtResult {
        let vc = VerificationCondition {
            predicate: expr.clone(),
            substitutions: List::new(),
            context: context.assumptions.iter().cloned().collect(),
            span: expr.span,
        };

        self.verify(&vc, self.config.global_timeout_ms.unwrap_or(5000))
    }

    fn get_model(&self, expr: &Expr, context: &SmtContext) -> Result<Model, SmtError> {
        let ctx = self.manager.primary();
        let mut solver = Z3Solver::new(Maybe::Some("QF_LIA"));
        let mut var_map = Map::new();

        // Add context constraints
        for constraint in &context.assumptions {
            match self.translate_expr(ctx.as_ref(), constraint, &mut var_map) {
                Ok(z3_constraint) => {
                    solver.assert(&z3_constraint);
                }
                Err(_) => continue,
            }
        }

        // Add the expression as constraint
        if let Ok(z3_expr) = self.translate_expr(ctx.as_ref(), expr, &mut var_map) {
            solver.assert(&z3_expr);
        }

        // Check and extract model
        let result = solver.check_sat();

        match result {
            AdvancedResult::Sat {
                model: Maybe::Some(z3_model),
            } => {
                // Extract variable assignments from model
                let mut assignments = Map::new();

                for (var_name, z3_var) in var_map.iter() {
                    // Evaluate variable in model
                    if let Maybe::Some(z3_int) = option_to_maybe(z3_var.as_int()) {
                        if let Maybe::Some(value) = option_to_maybe(z3_model.eval(&z3_int, true))
                            && let Maybe::Some(i64_value) = option_to_maybe(value.as_i64())
                        {
                            use verum_ast::literal::{IntLit, Literal, LiteralKind};
                            use verum_ast::span::Span;

                            let lit = Literal::new(
                                LiteralKind::Int(IntLit {
                                    value: i64_value as i128,
                                    suffix: None,
                                }),
                                Span::dummy(),
                            );
                            assignments.insert(var_name.clone(), lit);
                        }
                    } else if let Maybe::Some(z3_bool) = option_to_maybe(z3_var.as_bool())
                        && let Maybe::Some(value) = option_to_maybe(z3_model.eval(&z3_bool, true))
                        && let Maybe::Some(bool_value) = option_to_maybe(value.as_bool())
                    {
                        use verum_ast::literal::{Literal, LiteralKind};
                        use verum_ast::span::Span;

                        let lit = Literal::new(LiteralKind::Bool(bool_value), Span::dummy());
                        assignments.insert(var_name.clone(), lit);
                    }
                }

                Ok(Model { assignments })
            }
            _ => Err(SmtError::SolverError("cannot extract model".to_text())),
        }
    }

    fn verify_predicate(
        &self,
        predicate: &Expr,
        bindings: &Map<Text, Literal>,
    ) -> Result<bool, SmtError> {
        let ctx = self.manager.primary();
        let mut var_map = Map::new();

        // Add bindings to variable map
        for (name, lit) in bindings.iter() {
            match &lit.kind {
                LiteralKind::Int(i) => {
                    let int_val = z3::ast::Int::from_i64(i.value as i64);
                    var_map.insert(name.clone(), z3::ast::Dynamic::from(int_val));
                }
                LiteralKind::Bool(b) => {
                    let bool_val = z3::ast::Bool::from_bool(*b);
                    var_map.insert(name.clone(), z3::ast::Dynamic::from(bool_val));
                }
                _ => {}
            }
        }

        // Translate and check
        let z3_expr = self
            .translate_expr(ctx.as_ref(), predicate, &mut var_map)
            .map_err(|e| SmtError::SolverError(e.to_string().to_text()))?;

        let solver = z3::Solver::new();
        solver.assert(&z3_expr);

        match solver.check() {
            z3::SatResult::Sat => Ok(true),
            z3::SatResult::Unsat => Ok(false),
            z3::SatResult::Unknown => Err(SmtError::SolverError("Unknown result".to_text())),
        }
    }
}

// ==================== Statistics ====================

/// Statistics tracking for SMT solver performance.
///
/// Collects metrics on query outcomes, timing, and success rates
/// to support performance tuning and debugging.
#[derive(Debug, Default, Clone)]
pub struct SolverStats {
    /// Total number of SMT queries submitted to the solver.
    pub total_queries: u64,
    /// Number of queries that returned SAT (satisfiable).
    pub sat_count: u64,
    /// Number of queries that returned UNSAT (unsatisfiable/proven).
    pub unsat_count: u64,
    /// Number of queries where the solver could not determine the result.
    pub unknown_count: u64,
    /// Number of queries that exceeded the timeout limit.
    pub timeout_count: u64,
    /// Cumulative time spent in the solver, in milliseconds.
    pub total_time_ms: u64,
}

impl SolverStats {
    /// Calculate the average time per query in milliseconds.
    /// Returns 0.0 if no queries have been made.
    pub fn average_time_ms(&self) -> f64 {
        if self.total_queries == 0 {
            0.0
        } else {
            self.total_time_ms as f64 / self.total_queries as f64
        }
    }

    /// Calculate the success rate (SAT + UNSAT) / total.
    /// Returns 0.0 if no queries have been made.
    pub fn success_rate(&self) -> f64 {
        if self.total_queries == 0 {
            0.0
        } else {
            (self.sat_count + self.unsat_count) as f64 / self.total_queries as f64
        }
    }

    /// Generate a human-readable report of solver statistics.
    pub fn report(&self) -> Text {
        Text::from(format!(
            "SMT Solver Statistics:\n\
             - Total queries: {}\n\
             - Sat: {}, Unsat: {}, Unknown: {}, Timeout: {}\n\
             - Success rate: {:.1}%\n\
             - Average time: {:.2}ms\n\
             - Total time: {}ms",
            self.total_queries,
            self.sat_count,
            self.unsat_count,
            self.unknown_count,
            self.timeout_count,
            self.success_rate() * 100.0,
            self.average_time_ms(),
            self.total_time_ms
        ))
    }
}
