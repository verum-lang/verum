//! Refinement Evidence Propagation System
//!
//! Flow-sensitive refinement evidence tracking: maintains proof witnesses for satisfied
//! refinement predicates, propagates evidence through control flow (if/match narrowing),
//! and enables zero-cost refinement checks when evidence is available.
//!
//! This module implements flow-sensitive refinement tracking, enabling the type
//! checker to learn and propagate refinement predicates through control flow.
//!
//! # Problem
//!
//! Without evidence propagation, the compiler cannot reason about values after
//! conditional checks:
//!
//! ```verum
//! fn process(data: List<Int>) -> Int {
//!     if data.is_empty() { return 0; }
//!     // Without evidence propagation: compiler doesn't know data is non-empty
//!     first(data)  // May generate spurious error
//! }
//! ```
//!
//! # Solution
//!
//! Track refinement evidence through control flow:
//! 1. After `if cond { return/break/continue }`, we know `!cond` holds
//! 2. In `if cond { ... }` then-branch, we know `cond` holds
//! 3. In match arms, we know the pattern matched
//!
//! # Architecture
//!
//! - `PathCondition`: A predicate known to be true on current path
//! - `RefinementEvidence`: Maps variables to learned predicates
//! - `EvidenceStack`: Stack of evidence scopes for nested control flow
//!
//! # Performance
//!
//! - Evidence lookup: O(1) hash map
//! - Evidence propagation: O(n) where n = active conditions
//! - Memory: ~64 bytes per tracked variable

use std::collections::HashMap;
use std::fmt::{self, Display};

use verum_ast::{
    expr::{BinOp, Expr, ExprKind, UnOp},
    literal::{Literal, LiteralKind},
    span::Span,
    ty::{Ident, Path, PathSegment},
};
use verum_common::{Heap, List, Map, Maybe, Set, Text};

// ============================================================================
// PATH CONDITION
// ============================================================================

/// A predicate known to be true on the current execution path.
///
/// Path conditions are accumulated as we traverse control flow:
/// - After `if cond { return }`, we add `!cond` as a path condition
/// - In `if cond { ... }`, we add `cond` in the then-branch
/// - In match arms, we add the pattern constraint
#[derive(Debug, Clone)]
pub struct PathCondition {
    /// The predicate expression (must evaluate to Bool)
    pub predicate: Expr,
    /// The variable this condition constrains (if identifiable)
    pub constrained_var: Maybe<Text>,
    /// Source location where this condition was learned
    pub source_span: Span,
    /// Kind of condition (for diagnostics)
    pub kind: PathConditionKind,
}

/// Classification of path condition sources
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathConditionKind {
    /// Condition from if-expression (positive in then-branch)
    IfCondition,
    /// Negated condition after early exit (return/break/continue)
    NegatedAfterExit,
    /// Pattern match constraint
    PatternMatch,
    /// Explicit assume annotation
    Assume,
    /// Method call result (e.g., is_some() → Maybe::Some)
    MethodResult,
    /// User-provided proof
    UserProof,
}

impl PathCondition {
    /// Create a path condition from an if-expression condition
    pub fn from_if_condition(condition: &Expr, negated: bool, span: Span) -> Self {
        let predicate = if negated {
            Self::negate_expr(condition)
        } else {
            condition.clone()
        };

        let constrained_var = Self::extract_constrained_variable(&predicate);

        Self {
            predicate,
            constrained_var,
            source_span: span,
            kind: if negated {
                PathConditionKind::NegatedAfterExit
            } else {
                PathConditionKind::IfCondition
            },
        }
    }

    /// Create a path condition from a pattern match
    pub fn from_pattern_match(var_name: Text, pattern_predicate: Expr, span: Span) -> Self {
        Self {
            predicate: pattern_predicate,
            constrained_var: Maybe::Some(var_name),
            source_span: span,
            kind: PathConditionKind::PatternMatch,
        }
    }

    /// Create a path condition from a method result (e.g., is_empty() returns false)
    pub fn from_method_result(
        receiver_var: Text,
        method_name: &str,
        result_negated: bool,
        span: Span,
    ) -> Self {
        // Create expression: receiver.method_name() or !receiver.method_name()
        let receiver_expr = Expr::ident(Ident::new(receiver_var.clone(), span));
        let method_call = Expr::new(
            ExprKind::MethodCall {
                receiver: Heap::new(receiver_expr),
                method: Ident::new(method_name, span),
                type_args: List::new(),
                args: List::new(),
            },
            span,
        );

        let predicate = if result_negated {
            Self::negate_expr(&method_call)
        } else {
            method_call
        };

        Self {
            predicate,
            constrained_var: Maybe::Some(receiver_var),
            source_span: span,
            kind: PathConditionKind::MethodResult,
        }
    }

    /// Negate an expression (create `!expr`)
    /// Public static method for use by type narrowing.
    pub fn negate_expr_static(expr: &Expr) -> Expr {
        Self::negate_expr(expr)
    }

    /// Negate an expression (create `!expr`)
    fn negate_expr(expr: &Expr) -> Expr {
        // Simplify double negation
        if let ExprKind::Unary {
            op: UnOp::Not,
            expr: inner,
        } = &expr.kind
        {
            return (**inner).clone();
        }

        // Simplify negation of comparisons
        if let ExprKind::Binary { op, left, right } = &expr.kind {
            let negated_op = match op {
                BinOp::Eq => Some(BinOp::Ne),
                BinOp::Ne => Some(BinOp::Eq),
                BinOp::Lt => Some(BinOp::Ge),
                BinOp::Le => Some(BinOp::Gt),
                BinOp::Gt => Some(BinOp::Le),
                BinOp::Ge => Some(BinOp::Lt),
                _ => None,
            };

            if let Some(new_op) = negated_op {
                return Expr::new(
                    ExprKind::Binary {
                        op: new_op,
                        left: left.clone(),
                        right: right.clone(),
                    },
                    expr.span,
                );
            }
        }

        // Default: wrap in Not
        Expr::new(
            ExprKind::Unary {
                op: UnOp::Not,
                expr: Heap::new(expr.clone()),
            },
            expr.span,
        )
    }

    /// Extract the variable being constrained (if identifiable)
    fn extract_constrained_variable(expr: &Expr) -> Maybe<Text> {
        match &expr.kind {
            // Direct identifier: x
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    if let PathSegment::Name(ident) = &path.segments[0] {
                        return Maybe::Some(ident.name.clone());
                    }
                }
                Maybe::None
            }

            // Method call: x.is_empty()
            ExprKind::MethodCall { receiver, .. } => Self::extract_constrained_variable(receiver),

            // Binary comparison: x > 0
            ExprKind::Binary { left, .. } => Self::extract_constrained_variable(left),

            // Unary: !x.is_empty()
            ExprKind::Unary { expr: inner, .. } => Self::extract_constrained_variable(inner),

            // Field access: x.field
            ExprKind::Field { expr: inner, .. } => Self::extract_constrained_variable(inner),

            _ => Maybe::None,
        }
    }

    /// Check if this condition is about a specific variable
    pub fn constrains_variable(&self, var_name: &Text) -> bool {
        match &self.constrained_var {
            Maybe::Some(v) => v == var_name,
            Maybe::None => false,
        }
    }

    /// Check if this is a non-empty check (e.g., !data.is_empty())
    pub fn is_non_empty_check(&self) -> bool {
        match &self.predicate.kind {
            ExprKind::Unary {
                op: UnOp::Not,
                expr,
            } => {
                if let ExprKind::MethodCall { method, .. } = &expr.kind {
                    method.name.as_str() == "is_empty"
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Check if this is a Some/Ok check
    pub fn is_some_or_ok_check(&self) -> bool {
        match &self.predicate.kind {
            ExprKind::MethodCall { method, .. } => {
                matches!(method.name.as_str(), "is_some" | "is_ok")
            }
            _ => false,
        }
    }
}

// ============================================================================
// REFINEMENT EVIDENCE
// ============================================================================

/// Tracks learned refinement predicates for variables on the current path.
///
/// This is the main data structure for flow-sensitive refinement tracking.
/// It maintains a stack of evidence scopes that correspond to nested control flow.
#[derive(Debug, Clone)]
pub struct RefinementEvidence {
    /// Stack of evidence scopes (innermost scope is last)
    scopes: Vec<EvidenceScope>,
    /// Statistics for debugging/optimization
    stats: EvidenceStats,
}

/// A scope of evidence (corresponds to a control flow scope)
#[derive(Debug, Clone, Default)]
struct EvidenceScope {
    /// Path conditions active in this scope
    conditions: Vec<PathCondition>,
    /// Variables with learned predicates in this scope
    /// Maps variable name → list of learned predicates
    variable_predicates: HashMap<Text, Vec<PathCondition>>,
}

/// Statistics for evidence tracking
#[derive(Debug, Clone, Default)]
struct EvidenceStats {
    /// Total conditions tracked
    conditions_added: usize,
    /// Conditions used in verification
    conditions_used: usize,
    /// Cache hits when looking up evidence
    cache_hits: usize,
}

impl Default for RefinementEvidence {
    fn default() -> Self {
        Self::new()
    }
}

impl RefinementEvidence {
    /// Create a new empty evidence tracker
    pub fn new() -> Self {
        Self {
            scopes: vec![EvidenceScope::default()],
            stats: EvidenceStats::default(),
        }
    }

    /// Push a new scope (entering nested control flow)
    pub fn push_scope(&mut self) {
        self.scopes.push(EvidenceScope::default());
    }

    /// Pop a scope (leaving nested control flow)
    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    /// Add a path condition to the current scope
    pub fn add_condition(&mut self, condition: PathCondition) {
        self.stats.conditions_added += 1;

        if let Maybe::Some(ref var_name) = condition.constrained_var {
            // Also add to variable-specific index
            if let Some(scope) = self.scopes.last_mut() {
                scope
                    .variable_predicates
                    .entry(var_name.clone())
                    .or_default()
                    .push(condition.clone());
            }
        }

        if let Some(scope) = self.scopes.last_mut() {
            scope.conditions.push(condition);
        }
    }

    /// Add evidence that a condition holds in the current scope
    pub fn add_evidence_from_condition(&mut self, condition: &Expr, span: Span) {
        let path_condition = PathCondition::from_if_condition(condition, false, span);
        self.add_condition(path_condition);
    }

    /// Add evidence that a condition does NOT hold (after early exit)
    pub fn add_negated_evidence(&mut self, condition: &Expr, span: Span) {
        let path_condition = PathCondition::from_if_condition(condition, true, span);
        self.add_condition(path_condition);
    }

    /// Add evidence from a method call result
    pub fn add_method_evidence(
        &mut self,
        receiver_var: Text,
        method_name: &str,
        result_negated: bool,
        span: Span,
    ) {
        let condition =
            PathCondition::from_method_result(receiver_var, method_name, result_negated, span);
        self.add_condition(condition);
    }

    /// Get all conditions that constrain a specific variable
    pub fn get_variable_evidence(&mut self, var_name: &Text) -> Vec<&PathCondition> {
        self.stats.conditions_used += 1;

        let mut result = Vec::new();
        for scope in &self.scopes {
            if let Some(conditions) = scope.variable_predicates.get(var_name) {
                result.extend(conditions.iter());
            }
        }
        result
    }

    /// Get all active path conditions
    pub fn get_all_conditions(&self) -> Vec<&PathCondition> {
        self.scopes
            .iter()
            .flat_map(|scope| scope.conditions.iter())
            .collect()
    }

    /// Check if we have evidence that a variable is non-empty
    pub fn has_non_empty_evidence(&mut self, var_name: &Text) -> bool {
        for condition in self.get_variable_evidence(var_name) {
            if condition.is_non_empty_check() {
                self.stats.cache_hits += 1;
                return true;
            }
        }
        false
    }

    /// Check if we have evidence that a variable is Some/Ok
    pub fn has_some_or_ok_evidence(&mut self, var_name: &Text) -> bool {
        for condition in self.get_variable_evidence(var_name) {
            if condition.is_some_or_ok_check() {
                self.stats.cache_hits += 1;
                return true;
            }
        }
        false
    }

    /// Convert all evidence to SMT assumptions
    ///
    /// Returns a list of expressions that can be added to SMT path conditions
    pub fn to_smt_assumptions(&self) -> List<Expr> {
        self.get_all_conditions()
            .into_iter()
            .map(|c| c.predicate.clone())
            .collect()
    }

    /// Get statistics for debugging
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.stats.conditions_added,
            self.stats.conditions_used,
            self.stats.cache_hits,
        )
    }

    /// Clear all evidence (for new function)
    pub fn clear(&mut self) {
        self.scopes.clear();
        self.scopes.push(EvidenceScope::default());
    }
}

// ============================================================================
// EVIDENCE PROPAGATOR
// ============================================================================

/// Logic for propagating refinement evidence through control flow.
///
/// This struct contains the algorithms for determining what evidence
/// to add based on control flow patterns.
pub struct EvidencePropagator;

impl EvidencePropagator {
    /// Analyze an if-condition and extract evidence for both branches.
    ///
    /// Returns:
    /// - `then_evidence`: Predicates known true in then-branch
    /// - `else_evidence`: Predicates known true in else-branch (or continuation)
    pub fn analyze_if_condition(
        condition: &Expr,
        span: Span,
    ) -> (Vec<PathCondition>, Vec<PathCondition>) {
        let then_evidence = vec![PathCondition::from_if_condition(condition, false, span)];
        let else_evidence = vec![PathCondition::from_if_condition(condition, true, span)];

        (then_evidence, else_evidence)
    }

    /// Check if a block unconditionally exits (return, break, continue, panic).
    ///
    /// If true, we can propagate negated evidence to the continuation.
    pub fn block_unconditionally_exits(block: &verum_ast::expr::Block) -> bool {
        use verum_ast::stmt::StmtKind;

        // Check final expression
        if let Maybe::Some(ref expr) = block.expr {
            if Self::expr_unconditionally_exits(expr) {
                return true;
            }
        }

        // Check last statement
        if let Some(last_stmt) = block.stmts.last() {
            match &last_stmt.kind {
                StmtKind::Expr { expr, .. } => {
                    return Self::expr_unconditionally_exits(expr);
                }
                _ => {}
            }
        }

        false
    }

    /// Check if an expression unconditionally exits
    fn expr_unconditionally_exits(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Return(_) | ExprKind::Break { .. } | ExprKind::Continue { .. } => true,

            // Check for panic, unreachable, etc.
            ExprKind::Call { func, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    if path.segments.len() == 1 {
                        if let PathSegment::Name(ident) = &path.segments[0] {
                            return matches!(
                                ident.name.as_str(),
                                "panic" | "unreachable" | "todo" | "unimplemented"
                            );
                        }
                    }
                }
                false
            }

            ExprKind::Block(block) => Self::block_unconditionally_exits(block),

            _ => false,
        }
    }

    /// Extract variable name from a method call expression if it's a simple identifier.
    pub fn extract_receiver_variable(expr: &Expr) -> Maybe<Text> {
        match &expr.kind {
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    if let PathSegment::Name(ident) = &path.segments[0] {
                        return Maybe::Some(ident.name.clone());
                    }
                }
                Maybe::None
            }
            ExprKind::Paren(inner) => Self::extract_receiver_variable(inner),
            _ => Maybe::None,
        }
    }

    /// Analyze a method call condition (e.g., `data.is_empty()`)
    ///
    /// Returns the receiver variable name and method name if identifiable.
    pub fn analyze_method_condition(condition: &Expr) -> Maybe<(Text, Text, bool)> {
        match &condition.kind {
            // Direct method call: data.is_empty()
            ExprKind::MethodCall {
                receiver, method, ..
            } => {
                if let Maybe::Some(var_name) = Self::extract_receiver_variable(receiver) {
                    return Maybe::Some((var_name, method.name.clone(), false));
                }
                Maybe::None
            }

            // Negated method call: !data.is_empty()
            ExprKind::Unary {
                op: UnOp::Not,
                expr,
            } => {
                if let ExprKind::MethodCall {
                    receiver, method, ..
                } = &expr.kind
                {
                    if let Maybe::Some(var_name) = Self::extract_receiver_variable(receiver) {
                        return Maybe::Some((var_name, method.name.clone(), true));
                    }
                }
                Maybe::None
            }

            _ => Maybe::None,
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::span::FileId;

    fn test_span() -> Span {
        Span::new(0, 10, FileId::new(0))
    }

    fn make_ident_expr(name: &str) -> Expr {
        Expr::ident(Ident::new(name, test_span()))
    }

    fn make_method_call(receiver: &str, method: &str) -> Expr {
        Expr::new(
            ExprKind::MethodCall {
                receiver: Heap::new(make_ident_expr(receiver)),
                method: Ident::new(method, test_span()),
                type_args: List::new(),
                args: List::new(),
            },
            test_span(),
        )
    }

    #[test]
    fn test_evidence_scope_management() {
        let mut evidence = RefinementEvidence::new();

        // Add condition in outer scope
        let cond1 = PathCondition::from_if_condition(
            &make_method_call("data", "is_empty"),
            true,
            test_span(),
        );
        evidence.add_condition(cond1);

        assert_eq!(evidence.get_all_conditions().len(), 1);

        // Push scope and add another condition
        evidence.push_scope();
        let cond2 = PathCondition::from_if_condition(
            &make_method_call("other", "is_some"),
            false,
            test_span(),
        );
        evidence.add_condition(cond2);

        assert_eq!(evidence.get_all_conditions().len(), 2);

        // Pop scope - only outer condition remains
        evidence.pop_scope();
        assert_eq!(evidence.get_all_conditions().len(), 1);
    }

    #[test]
    fn test_variable_evidence_lookup() {
        let mut evidence = RefinementEvidence::new();

        // Add evidence for "data"
        evidence.add_method_evidence("data".into(), "is_empty", true, test_span());

        // Add evidence for "other"
        evidence.add_method_evidence("other".into(), "is_some", false, test_span());

        // Check lookup
        let data_evidence = evidence.get_variable_evidence(&"data".into());
        assert_eq!(data_evidence.len(), 1);
        assert!(data_evidence[0].is_non_empty_check());

        let other_evidence = evidence.get_variable_evidence(&"other".into());
        assert_eq!(other_evidence.len(), 1);
        assert!(other_evidence[0].is_some_or_ok_check());

        // Non-existent variable
        let none_evidence = evidence.get_variable_evidence(&"none".into());
        assert_eq!(none_evidence.len(), 0);
    }

    #[test]
    fn test_non_empty_evidence() {
        let mut evidence = RefinementEvidence::new();

        // Without evidence
        assert!(!evidence.has_non_empty_evidence(&"data".into()));

        // Add non-empty evidence
        evidence.add_method_evidence("data".into(), "is_empty", true, test_span());

        // Now should have evidence
        assert!(evidence.has_non_empty_evidence(&"data".into()));
    }

    #[test]
    fn test_negate_expr_simplification() {
        let span = test_span();

        // Negating a comparison should flip the operator
        let lt_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Lt,
                left: Heap::new(make_ident_expr("x")),
                right: Heap::new(Expr::literal(Literal::int(5, span))),
            },
            span,
        );

        let negated = PathCondition::negate_expr(&lt_expr);
        if let ExprKind::Binary { op, .. } = &negated.kind {
            assert_eq!(*op, BinOp::Ge);
        } else {
            panic!("Expected Binary expression");
        }

        // Double negation should simplify
        let not_expr = Expr::new(
            ExprKind::Unary {
                op: UnOp::Not,
                expr: Heap::new(make_ident_expr("flag")),
            },
            span,
        );

        let double_negated = PathCondition::negate_expr(&not_expr);
        if let ExprKind::Path(_) = &double_negated.kind {
            // Good - simplified to just the identifier
        } else {
            panic!("Expected simplified Path, got {:?}", double_negated.kind);
        }
    }

    #[test]
    fn test_analyze_method_condition() {
        // Simple method call: data.is_empty()
        let method_call = make_method_call("data", "is_empty");
        let result = EvidencePropagator::analyze_method_condition(&method_call);
        assert!(result.is_some());
        let (var, method, negated) = result.unwrap();
        assert_eq!(var.as_str(), "data");
        assert_eq!(method.as_str(), "is_empty");
        assert!(!negated);

        // Negated method call: !data.is_empty()
        let negated_call = Expr::new(
            ExprKind::Unary {
                op: UnOp::Not,
                expr: Heap::new(method_call.clone()),
            },
            test_span(),
        );
        let result = EvidencePropagator::analyze_method_condition(&negated_call);
        assert!(result.is_some());
        let (var, method, negated) = result.unwrap();
        assert_eq!(var.as_str(), "data");
        assert_eq!(method.as_str(), "is_empty");
        assert!(negated);
    }

    #[test]
    fn test_to_smt_assumptions() {
        let mut evidence = RefinementEvidence::new();

        evidence.add_method_evidence("data".into(), "is_empty", true, test_span());
        evidence.add_method_evidence("result".into(), "is_ok", false, test_span());

        let assumptions = evidence.to_smt_assumptions();
        assert_eq!(assumptions.len(), 2);
    }
}
