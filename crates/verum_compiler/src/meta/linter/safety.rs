//! Safety Pattern Detection for Meta Linter
//!
//! Detects general safety issues in meta code:
//! - String concatenation with external input
//! - Unbounded recursion
//! - Unbounded loops
//! - Panic/unwrap usage
//! - Non-deterministic operations
//!
//! Meta linter: static analysis of meta code for unsafe patterns (unbounded
//! recursion, infinite loops, unsafe interpolation without @safe attribute).

use std::collections::HashSet;

use verum_ast::decl::{FunctionBody, FunctionDecl};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::stmt::{Stmt, StmtKind};
use verum_ast::Span;
use verum_common::{Maybe, Text};

use super::dataflow::{AnalysisContext, ExternalInputChecker};
use super::patterns::UnsafePatternKind;
use super::results::{LintResult, UnsafePattern};

/// Safety pattern detector for meta code
pub struct SafetyDetector {
    /// Forbidden functions in meta code
    forbidden_functions: HashSet<String>,
}

impl SafetyDetector {
    /// Create a new safety detector with default forbidden functions
    pub fn new() -> Self {
        let mut forbidden = HashSet::new();
        forbidden.insert("println!".to_string());
        forbidden.insert("print!".to_string());
        forbidden.insert("panic!".to_string());
        forbidden.insert("todo!".to_string());
        forbidden.insert("unimplemented!".to_string());

        Self {
            forbidden_functions: forbidden,
        }
    }

    /// Create a safety detector with custom forbidden functions
    pub fn with_forbidden(forbidden: HashSet<String>) -> Self {
        Self {
            forbidden_functions: forbidden,
        }
    }

    /// Check for string concatenation with external input
    pub fn check_string_concat(
        &self,
        op: &verum_ast::BinOp,
        left: &Expr,
        right: &Expr,
        ctx: &AnalysisContext,
        span: Span,
        result: &mut LintResult,
    ) {
        // Only + operator can be string concat
        if matches!(op, verum_ast::BinOp::Add) {
            let left_external = ExternalInputChecker::expr_uses_external(left, ctx);
            let right_external = ExternalInputChecker::expr_uses_external(right, ctx);

            if left_external || right_external {
                Self::detect_pattern(
                    UnsafePatternKind::StringConcatenation,
                    Text::from("String concatenation with external input may cause injection"),
                    span,
                    Maybe::Some(Text::from("Use parameterized queries or proper escaping")),
                    result,
                );
            }
        }
    }

    /// Check for forbidden function usage
    pub fn check_forbidden_function(&self, func_name: &str, span: Span, result: &mut LintResult) {
        if self.forbidden_functions.contains(func_name) {
            Self::detect_pattern(
                UnsafePatternKind::PanicPossible,
                Text::from(format!("Use of forbidden function: {}", func_name)),
                span,
                Maybe::Some(Text::from("Remove or replace with a safe alternative")),
                result,
            );
        }
    }

    /// Check for unwrap/expect usage
    pub fn check_unwrap_expect(&self, method_name: &str, span: Span, result: &mut LintResult) {
        if method_name == "unwrap" || method_name == "expect" {
            Self::detect_pattern(
                UnsafePatternKind::PanicPossible,
                Text::from(format!("{}() may panic at compile-time", method_name)),
                span,
                Maybe::Some(Text::from("Use pattern matching or ? operator instead")),
                result,
            );
        }
    }

    /// Check for non-deterministic operations
    pub fn check_non_deterministic(&self, method_name: &str, span: Span, result: &mut LintResult) {
        if method_name == "now" || method_name == "random" {
            Self::detect_pattern(
                UnsafePatternKind::NonDeterministic,
                Text::from(format!("{}() is non-deterministic", method_name)),
                span,
                Maybe::Some(Text::from(
                    "Meta code must be deterministic for reproducible builds",
                )),
                result,
            );
        }
    }

    /// Check for unbounded recursion
    pub fn check_unbounded_recursion(&self, ctx: &AnalysisContext, span: Span, result: &mut LintResult) {
        if let Some(ref func_name) = ctx.current_function {
            if ctx.has_recursion(func_name) {
                Self::detect_pattern(
                    UnsafePatternKind::UnboundedRecursion,
                    Text::from(
                        "Recursive function may lack proper base case or termination guarantee",
                    ),
                    span,
                    Maybe::Some(Text::from(
                        "Ensure function has a base case that prevents infinite recursion",
                    )),
                    result,
                );
            }
        }
    }

    /// Check for unbounded loop (loop {} without break)
    pub fn check_unbounded_loop(&self, has_break: bool, span: Span, result: &mut LintResult) {
        if !has_break {
            Self::detect_pattern(
                UnsafePatternKind::UnboundedLoop,
                Text::from(
                    "Unbounded loop without break may cause infinite compile-time execution",
                ),
                span,
                Maybe::Some(Text::from(
                    "Add a break statement or use a bounded iteration",
                )),
                result,
            );
        }
    }

    /// Check for while true without break
    pub fn check_while_true(&self, has_break: bool, span: Span, result: &mut LintResult) {
        if !has_break {
            Self::detect_pattern(
                UnsafePatternKind::UnboundedLoop,
                Text::from(
                    "while true loop without break may cause infinite compile-time execution",
                ),
                span,
                Maybe::Some(Text::from(
                    "Add a break condition or use bounded iteration",
                )),
                result,
            );
        }
    }

    /// Calculate cyclomatic complexity of a function
    pub fn calculate_complexity(&self, func: &FunctionDecl) -> usize {
        let mut complexity = 1; // Base complexity

        if let Some(body) = &func.body {
            complexity += self.count_decision_points_in_body(body);
        }

        complexity
    }

    /// Count decision points in a function body
    fn count_decision_points_in_body(&self, body: &FunctionBody) -> usize {
        match body {
            FunctionBody::Block(block) => {
                let mut count = 0;
                for stmt in &block.stmts {
                    count += self.count_decision_points_in_stmt(stmt);
                }
                if let Some(expr) = &block.expr {
                    count += self.count_decision_points_in_expr(expr);
                }
                count
            }
            FunctionBody::Expr(expr) => self.count_decision_points_in_expr(expr),
        }
    }

    /// Count decision points in a statement
    fn count_decision_points_in_stmt(&self, stmt: &Stmt) -> usize {
        match &stmt.kind {
            StmtKind::Let { value, .. } => value
                .as_ref()
                .map(|e| self.count_decision_points_in_expr(e))
                .unwrap_or(0),
            StmtKind::LetElse { value, .. } => self.count_decision_points_in_expr(value) + 1,
            StmtKind::Expr { expr, .. } => self.count_decision_points_in_expr(expr),
            _ => 0,
        }
    }

    /// Count decision points in an expression
    fn count_decision_points_in_expr(&self, expr: &Expr) -> usize {
        match &expr.kind {
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                let mut count = 1; // if itself is a decision point
                for stmt in &then_branch.stmts {
                    count += self.count_decision_points_in_stmt(stmt);
                }
                if let Some(then_expr) = &then_branch.expr {
                    count += self.count_decision_points_in_expr(then_expr);
                }
                if let Some(else_expr) = else_branch {
                    count += self.count_decision_points_in_expr(else_expr);
                }
                count
            }
            ExprKind::Match { arms, .. } => {
                let mut count = arms.len().saturating_sub(1); // Each arm except first
                for arm in arms.iter() {
                    count += self.count_decision_points_in_expr(&arm.body);
                    if arm.guard.is_some() {
                        count += 1;
                    }
                }
                count
            }
            ExprKind::While { body, .. } | ExprKind::Loop { body, .. } => {
                let mut count = 1; // Loop is a decision point
                for stmt in &body.stmts {
                    count += self.count_decision_points_in_stmt(stmt);
                }
                if let Some(body_expr) = &body.expr {
                    count += self.count_decision_points_in_expr(body_expr);
                }
                count
            }
            ExprKind::For { body, .. } => {
                let mut count = 1; // For loop is a decision point
                for stmt in &body.stmts {
                    count += self.count_decision_points_in_stmt(stmt);
                }
                if let Some(body_expr) = &body.expr {
                    count += self.count_decision_points_in_expr(body_expr);
                }
                count
            }
            ExprKind::Binary { op, left, right } => {
                let mut count = 0;
                // Short-circuit operators add decision points
                if matches!(op, verum_ast::BinOp::And | verum_ast::BinOp::Or) {
                    count += 1;
                }
                count += self.count_decision_points_in_expr(left);
                count += self.count_decision_points_in_expr(right);
                count
            }
            ExprKind::Try(_) => 1, // ? operator is a decision point
            ExprKind::Block(block) => {
                let mut count = 0;
                for stmt in &block.stmts {
                    count += self.count_decision_points_in_stmt(stmt);
                }
                if let Some(block_expr) = &block.expr {
                    count += self.count_decision_points_in_expr(block_expr);
                }
                count
            }
            _ => 0,
        }
    }

    /// Detect and record an unsafe pattern
    fn detect_pattern(
        kind: UnsafePatternKind,
        description: Text,
        span: Span,
        suggestion: Maybe<Text>,
        result: &mut LintResult,
    ) {
        result.is_safe = false;
        result.unsafe_patterns.push(UnsafePattern {
            kind,
            description,
            span,
            suggestion,
        });
    }
}

impl Default for SafetyDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forbidden_functions() {
        let detector = SafetyDetector::new();
        let mut result = LintResult::safe();

        detector.check_forbidden_function("panic!", Span::default(), &mut result);
        assert!(!result.is_safe);
        assert_eq!(result.unsafe_patterns.len(), 1);
    }

    #[test]
    fn test_unwrap_detection() {
        let detector = SafetyDetector::new();
        let mut result = LintResult::safe();

        detector.check_unwrap_expect("unwrap", Span::default(), &mut result);
        assert!(!result.is_safe);
    }

    #[test]
    fn test_non_deterministic() {
        let detector = SafetyDetector::new();
        let mut result = LintResult::safe();

        detector.check_non_deterministic("random", Span::default(), &mut result);
        assert!(!result.is_safe);
    }
}
