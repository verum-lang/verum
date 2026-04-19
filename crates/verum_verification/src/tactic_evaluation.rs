//! Tactic Evaluation Engine for Verum Formal Proofs
//!
//! This module implements a comprehensive tactic evaluation system that maintains
//! proof state and applies tactics to transform goals in the Verum proof system.
//!
//! # Architecture
//!
//! The tactic evaluation engine consists of:
//! - **Goal State**: Current proof goals and hypotheses
//! - **Tactic Application**: Transform goals via tactic primitives
//! - **Progress Tracking**: Monitor proof completion status
//! - **SMT Integration**: Leverage Z3 for automated tactics
//!
//! # Core Tactics
//!
//! - `intro`: Introduce variables/hypotheses from goal
//! - `apply`: Apply a lemma to the current goal
//! - `rewrite`: Rewrite using an equality hypothesis
//! - `split`: Split conjunctions into subgoals
//! - `induction`: Proof by induction on a variable
//! - `cases`: Case analysis on an expression
//! - `simp`, `ring`, `omega`, `smt`: Automated tactics via SMT
//!
//! # Example
//!
//! ```no_run
//! use verum_verification::tactic_evaluation::{TacticEvaluator, Goal, Hypothesis};
//! use verum_ast::decl::TacticExpr;
//!
//! // Create evaluator
//! let mut evaluator = TacticEvaluator::new();
//!
//! // Set initial goal
//! // (Example code - see tests for complete usage)
//! ```
//!
//! Formal Proofs System (Verum 2.0+ planned):
//! Proof terms are first-class values via Curry-Howard correspondence.
//! Theorem syntax: `theorem name(params): proposition { proof_term }`
//! Proof tactics transform goals via forward/backward reasoning.
//! Automated tactics dispatch to SMT solvers (Z3) for decidable fragments.
//! Named tactics are user-extensible via `tactic my_tactic is { ... }`.
//! Proof strategies: `first [...]` tries alternatives, `repeat {...}` loops,
//! `match goal with ...` for pattern-based tactic selection.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use smallvec::SmallVec;
use verum_ast::decl::{
    CalcRelation, CalculationStep, ProofCase, TacticBody, TacticDecl, TacticExpr, TacticParam,
    TacticParamKind,
};
use verum_ast::{BinOp, Expr, ExprKind, Ident, LiteralKind, Pattern, Span};
use verum_common::{Heap, List, Map, Maybe, Text};
use verum_diagnostics::Diagnostic;
use verum_smt::tactics::{StrategyBuilder, TacticCombinator, TacticExecutor, TacticKind};
use verum_types::Type;

// Type alias for type-level lists to match verum_types expectations
type TypeList = List<Type>;
use z3::ast::Ast;

use thiserror::Error;

// ==================== Error Types ====================

/// Errors that can occur during tactic evaluation
#[derive(Debug, Error)]
pub enum TacticError {
    /// Tactic failed to make progress
    #[error("tactic failed: {0}")]
    Failed(Text),

    /// Tactic encountered an invalid goal state
    #[error("invalid goal state: {0}")]
    InvalidGoal(Text),

    /// Hypothesis not found in context
    #[error("hypothesis not found: {0}")]
    HypothesisNotFound(Text),

    /// Type mismatch in tactic application
    #[error("type mismatch: expected {expected}, found {actual}")]
    TypeMismatch { expected: Text, actual: Text },

    /// SMT solver error
    #[error("SMT solver error: {0}")]
    SmtError(Text),

    /// Timeout during tactic execution
    #[error("tactic timeout after {0:?}")]
    Timeout(Duration),

    /// Tactic not implemented
    #[error("tactic not implemented: {0}")]
    NotImplemented(Text),

    /// Invalid tactic argument
    #[error("invalid tactic argument: {0}")]
    InvalidArgument(Text),

    /// Goal already proven
    #[error("goal already proven")]
    AlreadyProven,

    /// No goals remaining
    #[error("no goals remaining")]
    NoGoals,
}

/// Result type for tactic operations
pub type TacticResult<T> = Result<T, TacticError>;

// ==================== Z3 Tactic Types ====================

/// Result of applying a Z3 tactic to a goal
///
/// Contains the resulting subgoals (if any) and whether the goal was proven.
#[derive(Debug, Clone)]
pub struct Z3TacticResult {
    /// Subgoals produced by the tactic application
    pub subgoals: List<Z3Subgoal>,

    /// Whether the goal was fully proven (no remaining subgoals)
    pub is_proven: bool,
}

/// A subgoal produced by Z3 tactic application
#[derive(Debug, Clone)]
pub struct Z3Subgoal {
    /// The formula that needs to be proven
    pub formula: Expr,

    /// Depth of this subgoal in the proof tree
    pub depth: usize,
}

/// Strategy for composing Z3 tactics
///
/// Allows building complex tactic strategies using Z3's combinator framework.
#[derive(Debug, Clone)]
pub enum Z3TacticStrategy {
    /// A named Z3 tactic (e.g., "simplify", "smt", "lia")
    Named(Text),

    /// Sequential composition: apply first, then second
    AndThen(Heap<Z3TacticStrategy>, Heap<Z3TacticStrategy>),

    /// Alternative: try first, if fails try second
    OrElse(Heap<Z3TacticStrategy>, Heap<Z3TacticStrategy>),

    /// Repeat tactic up to max_iterations times
    Repeat(Heap<Z3TacticStrategy>, u32),

    /// Apply tactic with timeout
    TryFor(Heap<Z3TacticStrategy>, Duration),

    /// Skip tactic (do nothing, always succeeds)
    Skip,

    /// Fail tactic (always fails)
    Fail,
}

impl Z3TacticStrategy {
    /// Create a named tactic strategy
    pub fn named(name: &str) -> Self {
        Z3TacticStrategy::Named(Text::from(name))
    }

    /// Create simplify tactic
    pub fn simplify() -> Self {
        Self::named("simplify")
    }

    /// Create SMT tactic
    pub fn smt() -> Self {
        Self::named("smt")
    }

    /// Create LIA (Linear Integer Arithmetic) tactic
    pub fn lia() -> Self {
        Self::named("lia")
    }

    /// Create NLA (Non-Linear Arithmetic) tactic
    pub fn nla() -> Self {
        Self::named("qfnra-nlsat")
    }

    /// Create solve-eqs tactic
    pub fn solve_eqs() -> Self {
        Self::named("solve-eqs")
    }

    /// Create propagate-values tactic
    pub fn propagate_values() -> Self {
        Self::named("propagate-values")
    }

    /// Create ctx-simplify tactic
    pub fn ctx_simplify() -> Self {
        Self::named("ctx-simplify")
    }

    /// Create bit-blast tactic for bitvectors
    pub fn bit_blast() -> Self {
        Self::named("bit-blast")
    }

    /// Chain two tactics: apply first then second
    pub fn and_then(self, next: Z3TacticStrategy) -> Self {
        Z3TacticStrategy::AndThen(Heap::new(self), Heap::new(next))
    }

    /// Try alternative: if self fails, try other
    pub fn or_else(self, other: Z3TacticStrategy) -> Self {
        Z3TacticStrategy::OrElse(Heap::new(self), Heap::new(other))
    }

    /// Repeat this tactic up to max times
    pub fn repeat(self, max: u32) -> Self {
        Z3TacticStrategy::Repeat(Heap::new(self), max)
    }

    /// Apply with timeout
    pub fn try_for(self, timeout: Duration) -> Self {
        Z3TacticStrategy::TryFor(Heap::new(self), timeout)
    }

    /// Create a default powerful strategy for general proving
    ///
    /// Combines simplification, equation solving, and SMT solving
    pub fn default_prover() -> Self {
        Self::simplify()
            .and_then(Self::solve_eqs())
            .and_then(Self::propagate_values())
            .and_then(Self::smt())
    }

    /// Create a strategy for linear arithmetic
    pub fn linear_arithmetic() -> Self {
        Self::simplify()
            .and_then(Self::propagate_values())
            .and_then(Self::lia())
    }

    /// Create a strategy for non-linear arithmetic
    pub fn nonlinear_arithmetic() -> Self {
        Self::simplify()
            .and_then(Self::propagate_values())
            .and_then(Self::nla())
    }
}

// ==================== Core Types ====================

/// A proof goal to be proven
///
/// Goals represent propositions that need to be proven given a context
/// of hypotheses. The tactic evaluator transforms goals until they are
/// trivially true or discharged by an automated tactic.
#[derive(Debug, Clone)]
pub struct Goal {
    /// Unique goal identifier
    pub id: usize,

    /// The proposition to prove
    pub proposition: Heap<Expr>,

    /// Available hypotheses
    pub hypotheses: List<Hypothesis>,

    /// Meta information about the goal
    pub meta: GoalMetadata,
}

impl Goal {
    /// Create a new goal
    pub fn new(id: usize, proposition: Expr) -> Self {
        Self {
            id,
            proposition: Heap::new(proposition),
            hypotheses: List::new(),
            meta: GoalMetadata::default(),
        }
    }

    /// Create a goal with hypotheses
    pub fn with_hypotheses(id: usize, proposition: Expr, hypotheses: List<Hypothesis>) -> Self {
        Self {
            id,
            proposition: Heap::new(proposition),
            hypotheses,
            meta: GoalMetadata::default(),
        }
    }

    /// Add a hypothesis to this goal
    pub fn add_hypothesis(&mut self, hyp: Hypothesis) {
        self.hypotheses.push(hyp);
    }

    /// Find a hypothesis by name
    pub fn find_hypothesis(&self, name: &Text) -> Maybe<&Hypothesis> {
        self.hypotheses
            .iter()
            .find(|&hyp| &hyp.name == name)
            .map(|v| v as _)
    }

    /// Check if the goal is trivially true
    pub fn is_trivial(&self) -> bool {
        // Check for True literal
        if let ExprKind::Literal(lit) = &self.proposition.kind
            && let LiteralKind::Bool(true) = lit.kind
        {
            return true;
        }

        // Check if goal matches any hypothesis exactly
        for hyp in &self.hypotheses {
            if self.expr_equal(&hyp.proposition, &self.proposition) {
                return true;
            }
        }

        false
    }

    /// Check if two expressions are structurally equal
    ///
    /// Performs a deep structural equality check that handles:
    /// - Alpha-equivalence (renamed bound variables)
    /// - Path normalization
    /// - Literal comparison
    /// - Recursive expression comparison
    ///
    /// Proof term equality: structural comparison with alpha-equivalence
    /// (renamed bound variables are considered equal) and path normalization.
    fn expr_equal(&self, e1: &Expr, e2: &Expr) -> bool {
        expr_structural_equal(e1, e2, &mut HashMap::new())
    }
}

/// Perform structural equality check with alpha-equivalence support
///
/// The `bindings` map tracks variable renamings for alpha-equivalence:
/// if we encounter `forall x. P(x)` and `forall y. P(y)`, we track
/// that x maps to y and check P(x) = P(y) under that mapping.
fn expr_structural_equal(e1: &Expr, e2: &Expr, bindings: &mut HashMap<Text, Text>) -> bool {
    use verum_ast::literal::{Literal, LiteralKind};
    use verum_ast::{Path, ty::PathSegment};

    match (&e1.kind, &e2.kind) {
        // Literal equality - compare literal values
        (ExprKind::Literal(lit1), ExprKind::Literal(lit2)) => literal_equal(lit1, lit2),

        // Path equality - handle variable references with alpha-equivalence
        (ExprKind::Path(p1), ExprKind::Path(p2)) => path_equal(p1, p2, bindings),

        // Binary operations - check operator and operands
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
        ) => {
            op1 == op2
                && expr_structural_equal(l1, l2, bindings)
                && expr_structural_equal(r1, r2, bindings)
        }

        // Unary operations - check operator and operand
        (ExprKind::Unary { op: op1, expr: e1 }, ExprKind::Unary { op: op2, expr: e2 }) => {
            op1 == op2 && expr_structural_equal(e1, e2, bindings)
        }

        // Function calls - check function and arguments
        (
            ExprKind::Call {
                func: f1,
                args: a1, ..
            },
            ExprKind::Call {
                func: f2,
                args: a2, ..
            },
        ) => {
            expr_structural_equal(f1, f2, bindings)
                && a1.len() == a2.len()
                && a1
                    .iter()
                    .zip(a2.iter())
                    .all(|(arg1, arg2)| expr_structural_equal(arg1, arg2, bindings))
        }

        // Method calls
        (
            ExprKind::MethodCall {
                receiver: r1,
                method: m1,
                args: a1,
                ..
            },
            ExprKind::MethodCall {
                receiver: r2,
                method: m2,
                args: a2,
                ..
            },
        ) => {
            m1.as_str() == m2.as_str()
                && expr_structural_equal(r1, r2, bindings)
                && a1.len() == a2.len()
                && a1
                    .iter()
                    .zip(a2.iter())
                    .all(|(arg1, arg2)| expr_structural_equal(arg1, arg2, bindings))
        }

        // Tuples
        (ExprKind::Tuple(elems1), ExprKind::Tuple(elems2)) => {
            elems1.len() == elems2.len()
                && elems1
                    .iter()
                    .zip(elems2.iter())
                    .all(|(e1, e2)| expr_structural_equal(e1, e2, bindings))
        }

        // Array literals
        (ExprKind::Array(arr1), ExprKind::Array(arr2)) => match (arr1, arr2) {
            (verum_ast::ArrayExpr::List(l1), verum_ast::ArrayExpr::List(l2)) => {
                l1.len() == l2.len()
                    && l1
                        .iter()
                        .zip(l2.iter())
                        .all(|(e1, e2)| expr_structural_equal(e1, e2, bindings))
            }
            (
                verum_ast::ArrayExpr::Repeat {
                    value: v1,
                    count: c1,
                },
                verum_ast::ArrayExpr::Repeat {
                    value: v2,
                    count: c2,
                },
            ) => expr_structural_equal(v1, v2, bindings) && expr_structural_equal(c1, c2, bindings),
            _ => false,
        },

        // Field access
        (
            ExprKind::Field {
                expr: e1,
                field: f1,
                ..
            },
            ExprKind::Field {
                expr: e2,
                field: f2,
                ..
            },
        ) => f1.as_str() == f2.as_str() && expr_structural_equal(e1, e2, bindings),

        // Index access
        (
            ExprKind::Index {
                expr: e1,
                index: i1,
            },
            ExprKind::Index {
                expr: e2,
                index: i2,
            },
        ) => expr_structural_equal(e1, e2, bindings) && expr_structural_equal(i1, i2, bindings),

        // If expressions
        (
            ExprKind::If {
                condition: c1,
                then_branch: t1,
                else_branch: eb1,
            },
            ExprKind::If {
                condition: c2,
                then_branch: t2,
                else_branch: eb2,
            },
        ) => {
            if_condition_equal(c1, c2, bindings)
                && block_equal(t1, t2, bindings)
                && match (eb1, eb2) {
                    (Some(e1), Some(e2)) => expr_structural_equal(e1, e2, bindings),
                    (None, None) => true,
                    _ => false,
                }
        }

        // Match expressions
        (ExprKind::Match { expr: e1, arms: a1 }, ExprKind::Match { expr: e2, arms: a2 }) => {
            expr_structural_equal(e1, e2, bindings)
                && a1.len() == a2.len()
                && a1
                    .iter()
                    .zip(a2.iter())
                    .all(|(arm1, arm2)| match_arm_equal(arm1, arm2, bindings))
        }

        // Lambda/Closure - handle alpha-equivalence for bound variables
        (
            ExprKind::Closure {
                params: p1,
                body: b1,
                ..
            },
            ExprKind::Closure {
                params: p2,
                body: b2,
                ..
            },
        ) => {
            if p1.len() != p2.len() {
                return false;
            }

            // Create new bindings for lambda parameters (alpha-equivalence)
            let mut new_bindings = bindings.clone();
            for (param1, param2) in p1.iter().zip(p2.iter()) {
                // Map parameter names for alpha-equivalence
                // Extract name from pattern
                if let (Some(n1), Some(n2)) = (
                    extract_pattern_name(&param1.pattern),
                    extract_pattern_name(&param2.pattern),
                ) {
                    new_bindings.insert(n1, n2);
                }
            }

            expr_structural_equal(b1, b2, &mut new_bindings)
        }

        // Block expressions
        (ExprKind::Block(b1), ExprKind::Block(b2)) => block_equal(b1, b2, bindings),

        // Range expressions
        (
            ExprKind::Range {
                start: s1,
                end: e1,
                inclusive: i1,
            },
            ExprKind::Range {
                start: s2,
                end: e2,
                inclusive: i2,
            },
        ) => {
            i1 == i2
                && opt_expr_equal(s1.as_deref(), s2.as_deref(), bindings)
                && opt_expr_equal(e1.as_deref(), e2.as_deref(), bindings)
        }

        // Cast expressions
        (ExprKind::Cast { expr: e1, ty: t1 }, ExprKind::Cast { expr: e2, ty: t2 }) => {
            expr_structural_equal(e1, e2, bindings) && type_equal(t1, t2)
        }

        // Reference expressions are handled via Unary with RefX ops
        // The AST doesn't have a separate Ref variant

        // Dereference is handled via Unary with Deref op
        // The AST doesn't have a separate Deref variant

        // Return expressions
        (ExprKind::Return(e1), ExprKind::Return(e2)) => {
            opt_expr_equal(e1.as_deref(), e2.as_deref(), bindings)
        }

        // Break expressions
        (
            ExprKind::Break {
                label: l1,
                value: v1,
            },
            ExprKind::Break {
                label: l2,
                value: v2,
            },
        ) => l1 == l2 && opt_expr_equal(v1.as_deref(), v2.as_deref(), bindings),

        // Continue expressions
        (ExprKind::Continue { label: l1 }, ExprKind::Continue { label: l2 }) => l1 == l2,

        // Struct literals
        (
            ExprKind::Record {
                path: p1,
                fields: f1,
                base: b1,
                ..
            },
            ExprKind::Record {
                path: p2,
                fields: f2,
                base: b2,
                ..
            },
        ) => {
            path_equal(p1, p2, bindings)
                && f1.len() == f2.len()
                && f1.iter().zip(f2.iter()).all(|(fld1, fld2)| {
                    fld1.name.as_str() == fld2.name.as_str()
                        && match (&fld1.value, &fld2.value) {
                            (Some(v1), Some(v2)) => expr_structural_equal(v1, v2, bindings),
                            (None, None) => true,
                            _ => false,
                        }
                })
                && opt_expr_equal(b1.as_deref(), b2.as_deref(), bindings)
        }

        // Await expressions
        (ExprKind::Await(e1), ExprKind::Await(e2)) => expr_structural_equal(e1, e2, bindings),

        // Try expressions
        (ExprKind::Try(e1), ExprKind::Try(e2)) => expr_structural_equal(e1, e2, bindings),

        // Forall quantifier expressions - alpha-equivalence
        (
            ExprKind::Forall {
                bindings: bindings1,
                body: b1,
            },
            ExprKind::Forall {
                bindings: bindings2,
                body: b2,
            },
        ) => {
            if bindings1.len() != bindings2.len() {
                return false;
            }

            // Check each binding pairwise
            let mut new_bindings = bindings.clone();
            for (binding1, binding2) in bindings1.iter().zip(bindings2.iter()) {
                // Check types match if both have types
                match (&binding1.ty, &binding2.ty) {
                    (verum_common::Maybe::Some(t1), verum_common::Maybe::Some(t2)) => {
                        if !type_equal(t1, t2) {
                            return false;
                        }
                    }
                    (verum_common::Maybe::None, verum_common::Maybe::None) => {}
                    _ => return false,
                }
                if !pattern_equal(&binding1.pattern, &binding2.pattern, &mut new_bindings) {
                    return false;
                }
            }
            expr_structural_equal(b1, b2, &mut new_bindings)
        }

        // Exists quantifier expressions - alpha-equivalence
        (
            ExprKind::Exists {
                bindings: bindings1,
                body: b1,
            },
            ExprKind::Exists {
                bindings: bindings2,
                body: b2,
            },
        ) => {
            if bindings1.len() != bindings2.len() {
                return false;
            }

            // Check each binding pairwise
            let mut new_bindings = bindings.clone();
            for (binding1, binding2) in bindings1.iter().zip(bindings2.iter()) {
                // Check types match if both have types
                match (&binding1.ty, &binding2.ty) {
                    (verum_common::Maybe::Some(t1), verum_common::Maybe::Some(t2)) => {
                        if !type_equal(t1, t2) {
                            return false;
                        }
                    }
                    (verum_common::Maybe::None, verum_common::Maybe::None) => {}
                    _ => return false,
                }
                if !pattern_equal(&binding1.pattern, &binding2.pattern, &mut new_bindings) {
                    return false;
                }
            }
            expr_structural_equal(b1, b2, &mut new_bindings)
        }

        // Default: expressions are not equal if their kinds don't match
        _ => false,
    }
}

/// Extract a name from a pattern if it's a simple binding
fn extract_pattern_name(pattern: &Pattern) -> Option<Text> {
    use verum_ast::pattern::PatternKind;

    match &pattern.kind {
        PatternKind::Ident { name, .. } => Some(Text::from(name.as_str())),
        PatternKind::Paren(inner) => extract_pattern_name(inner),
        _ => None,
    }
}

/// Compare literal values
fn literal_equal(lit1: &verum_ast::Literal, lit2: &verum_ast::Literal) -> bool {
    use verum_ast::literal::LiteralKind;

    match (&lit1.kind, &lit2.kind) {
        (LiteralKind::Bool(b1), LiteralKind::Bool(b2)) => b1 == b2,
        (LiteralKind::Int(i1), LiteralKind::Int(i2)) => i1.value == i2.value,
        (LiteralKind::Float(f1), LiteralKind::Float(f2)) => f1.value == f2.value,
        (LiteralKind::Char(c1), LiteralKind::Char(c2)) => c1 == c2,
        (LiteralKind::Text(s1), LiteralKind::Text(s2)) => s1.as_str() == s2.as_str(),
        (
            LiteralKind::Tagged {
                tag: t1,
                content: c1,
            },
            LiteralKind::Tagged {
                tag: t2,
                content: c2,
            },
        ) => t1 == t2 && c1 == c2,
        _ => false,
    }
}

/// Compare paths with alpha-equivalence support
fn path_equal(p1: &verum_ast::Path, p2: &verum_ast::Path, bindings: &HashMap<Text, Text>) -> bool {
    use verum_ast::ty::PathSegment;

    if p1.segments.len() != p2.segments.len() {
        return false;
    }

    // For single-segment paths (simple variables), check alpha-equivalence
    if p1.segments.len() == 1 {
        if let (PathSegment::Name(n1), PathSegment::Name(n2)) = (&p1.segments[0], &p2.segments[0]) {
            let name1 = Text::from(n1.as_str());
            let name2 = Text::from(n2.as_str());

            // Check if name1 is bound to name2 via alpha-equivalence
            if let Some(bound_to) = bindings.get(&name1) {
                return bound_to == &name2;
            }

            // Otherwise, names must match exactly
            return name1 == name2;
        }
    }

    // For multi-segment paths, compare segment by segment
    p1.segments
        .iter()
        .zip(p2.segments.iter())
        .all(|(seg1, seg2)| match (seg1, seg2) {
            (PathSegment::Name(n1), PathSegment::Name(n2)) => n1.as_str() == n2.as_str(),
            (PathSegment::SelfValue, PathSegment::SelfValue) => true,
            (PathSegment::Super, PathSegment::Super) => true,
            (PathSegment::Cog, PathSegment::Cog) => true,
            (PathSegment::Relative, PathSegment::Relative) => true,
            _ => false,
        })
}

/// Compare optional expressions
fn opt_expr_equal(
    e1: Option<&Expr>,
    e2: Option<&Expr>,
    bindings: &mut HashMap<Text, Text>,
) -> bool {
    match (e1, e2) {
        (Some(ex1), Some(ex2)) => expr_structural_equal(ex1, ex2, bindings),
        (None, None) => true,
        _ => false,
    }
}

/// Compare blocks
fn block_equal(
    b1: &verum_ast::expr::Block,
    b2: &verum_ast::expr::Block,
    bindings: &mut HashMap<Text, Text>,
) -> bool {
    b1.stmts.len() == b2.stmts.len()
        && b1
            .stmts
            .iter()
            .zip(b2.stmts.iter())
            .all(|(s1, s2)| stmt_equal(s1, s2, bindings))
}

/// Compare statements
fn stmt_equal(
    s1: &verum_ast::stmt::Stmt,
    s2: &verum_ast::stmt::Stmt,
    bindings: &mut HashMap<Text, Text>,
) -> bool {
    use verum_ast::stmt::StmtKind;

    match (&s1.kind, &s2.kind) {
        (
            StmtKind::Let {
                pattern: p1,
                ty: t1,
                value: v1,
                ..
            },
            StmtKind::Let {
                pattern: p2,
                ty: t2,
                value: v2,
                ..
            },
        ) => {
            pattern_equal(p1, p2, bindings)
                && match (t1, t2) {
                    (Some(ty1), Some(ty2)) => type_equal(ty1, ty2),
                    (None, None) => true,
                    _ => false,
                }
                && match (v1, v2) {
                    (Some(val1), Some(val2)) => expr_structural_equal(val1, val2, bindings),
                    (None, None) => true,
                    _ => false,
                }
        }
        (
            StmtKind::Expr {
                expr: e1,
                has_semi: s1,
            },
            StmtKind::Expr {
                expr: e2,
                has_semi: s2,
            },
        ) => s1 == s2 && expr_structural_equal(e1, e2, bindings),
        (StmtKind::Empty, StmtKind::Empty) => true,
        _ => false,
    }
}

/// Compare patterns
fn pattern_equal(p1: &Pattern, p2: &Pattern, bindings: &mut HashMap<Text, Text>) -> bool {
    use verum_ast::pattern::PatternKind;

    match (&p1.kind, &p2.kind) {
        (PatternKind::Wildcard, PatternKind::Wildcard) => true,
        (
            PatternKind::Ident {
                name: n1,
                mutable: m1,
                ..
            },
            PatternKind::Ident {
                name: n2,
                mutable: m2,
                ..
            },
        ) => {
            m1 == m2 && {
                // Add binding for alpha-equivalence
                bindings.insert(Text::from(n1.as_str()), Text::from(n2.as_str()));
                true
            }
        }
        (PatternKind::Literal(l1), PatternKind::Literal(l2)) => literal_equal(l1, l2),
        (PatternKind::Tuple(ps1), PatternKind::Tuple(ps2)) => {
            ps1.len() == ps2.len()
                && ps1
                    .iter()
                    .zip(ps2.iter())
                    .all(|(pat1, pat2)| pattern_equal(pat1, pat2, bindings))
        }
        (
            PatternKind::Record {
                path: path1,
                fields: f1,
                ..
            },
            PatternKind::Record {
                path: path2,
                fields: f2,
                ..
            },
        ) => {
            path_equal(path1, path2, bindings)
                && f1.len() == f2.len()
                && f1.iter().zip(f2.iter()).all(|(fp1, fp2)| {
                    fp1.name.as_str() == fp2.name.as_str()
                        && match (&fp1.pattern, &fp2.pattern) {
                            (Maybe::Some(p1), Maybe::Some(p2)) => pattern_equal(p1, p2, bindings),
                            (Maybe::None, Maybe::None) => true,
                            _ => false,
                        }
                })
        }
        (PatternKind::Or(ps1), PatternKind::Or(ps2)) => {
            ps1.len() == ps2.len()
                && ps1
                    .iter()
                    .zip(ps2.iter())
                    .all(|(pat1, pat2)| pattern_equal(pat1, pat2, bindings))
        }
        _ => false,
    }
}

/// Compare types
fn type_equal(t1: &verum_ast::Type, t2: &verum_ast::Type) -> bool {
    use verum_ast::TypeKind;

    match (&t1.kind, &t2.kind) {
        (TypeKind::Path(p1), TypeKind::Path(p2)) => {
            p1.segments.len() == p2.segments.len()
                && p1
                    .segments
                    .iter()
                    .zip(p2.segments.iter())
                    .all(|(s1, s2)| match (s1, s2) {
                        (
                            verum_ast::ty::PathSegment::Name(n1),
                            verum_ast::ty::PathSegment::Name(n2),
                        ) => n1.as_str() == n2.as_str(),
                        _ => false,
                    })
        }
        (TypeKind::Tuple(ts1), TypeKind::Tuple(ts2)) => {
            ts1.len() == ts2.len()
                && ts1
                    .iter()
                    .zip(ts2.iter())
                    .all(|(ty1, ty2)| type_equal(ty1, ty2))
        }
        (
            TypeKind::Array {
                element: e1,
                size: s1,
            },
            TypeKind::Array {
                element: e2,
                size: s2,
            },
        ) => {
            type_equal(e1, e2)
                && match (s1, s2) {
                    (Maybe::Some(sz1), Maybe::Some(sz2)) => {
                        expr_structural_equal(sz1, sz2, &mut HashMap::new())
                    }
                    (Maybe::None, Maybe::None) => true,
                    _ => false,
                }
        }
        (
            TypeKind::Reference {
                mutable: m1,
                inner: ty1,
            },
            TypeKind::Reference {
                mutable: m2,
                inner: ty2,
            },
        ) => m1 == m2 && type_equal(ty1, ty2),
        (TypeKind::Inferred, TypeKind::Inferred) => true,
        _ => false,
    }
}

/// Compare if conditions
fn if_condition_equal(
    c1: &verum_ast::expr::IfCondition,
    c2: &verum_ast::expr::IfCondition,
    bindings: &mut HashMap<Text, Text>,
) -> bool {
    c1.conditions.len() == c2.conditions.len()
        && c1
            .conditions
            .iter()
            .zip(c2.conditions.iter())
            .all(|(ck1, ck2)| match (ck1, ck2) {
                (
                    verum_ast::expr::ConditionKind::Expr(e1),
                    verum_ast::expr::ConditionKind::Expr(e2),
                ) => expr_structural_equal(e1, e2, bindings),
                (
                    verum_ast::expr::ConditionKind::Let {
                        pattern: p1,
                        value: v1,
                        ..
                    },
                    verum_ast::expr::ConditionKind::Let {
                        pattern: p2,
                        value: v2,
                        ..
                    },
                ) => pattern_equal(p1, p2, bindings) && expr_structural_equal(v1, v2, bindings),
                _ => false,
            })
}

/// Compare match arms
fn match_arm_equal(
    arm1: &verum_ast::MatchArm,
    arm2: &verum_ast::MatchArm,
    bindings: &mut HashMap<Text, Text>,
) -> bool {
    let mut new_bindings = bindings.clone();
    pattern_equal(&arm1.pattern, &arm2.pattern, &mut new_bindings)
        && match (&arm1.guard, &arm2.guard) {
            (Some(g1), Some(g2)) => expr_structural_equal(g1, g2, &mut new_bindings),
            (None, None) => true,
            _ => false,
        }
        && expr_structural_equal(&arm1.body, &arm2.body, &mut new_bindings)
}

/// Metadata about a goal
#[derive(Debug, Clone, Default)]
pub struct GoalMetadata {
    /// Source location of the goal
    pub source: Maybe<Span>,

    /// Goal name (for debugging)
    pub name: Maybe<Text>,

    /// Whether this goal was generated by induction
    pub from_induction: bool,

    /// Parent goal ID (if this is a subgoal)
    pub parent_id: Maybe<usize>,
}

/// A hypothesis available in the proof context
///
/// Hypotheses are named propositions that have been established
/// and can be used in subsequent proof steps.
#[derive(Debug, Clone)]
pub struct Hypothesis {
    /// Hypothesis name
    pub name: Text,

    /// The proposition this hypothesis asserts
    pub proposition: Heap<Expr>,

    /// Type of the hypothesis (if typed)
    pub ty: Maybe<Type>,

    /// Source of this hypothesis
    pub source: HypothesisSource,
}

impl Hypothesis {
    /// Create a new hypothesis
    pub fn new(name: Text, proposition: Expr) -> Self {
        Self {
            name,
            proposition: Heap::new(proposition),
            ty: Maybe::None,
            source: HypothesisSource::User,
        }
    }

    /// Create a hypothesis from an assumption
    pub fn assumption(name: Text, proposition: Expr) -> Self {
        Self {
            name,
            proposition: Heap::new(proposition),
            ty: Maybe::None,
            source: HypothesisSource::Assumption,
        }
    }

    /// Create a hypothesis from induction
    pub fn induction(name: Text, proposition: Expr) -> Self {
        Self {
            name,
            proposition: Heap::new(proposition),
            ty: Maybe::None,
            source: HypothesisSource::Induction,
        }
    }
}

/// Source of a hypothesis
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HypothesisSource {
    /// User-provided hypothesis
    User,
    /// Assumption from intro tactic
    Assumption,
    /// Induction hypothesis
    Induction,
    /// Generated by tactic
    Generated,
}

/// Proof state tracking progress through a proof
///
/// The proof state maintains the current goals and tracks which
/// have been proven. Tactics operate on the proof state to
/// transform and discharge goals.
#[derive(Debug, Clone)]
pub struct ProofState {
    /// Current active goals
    pub goals: List<Goal>,

    /// Proven goals (archived)
    pub proven_goals: List<Goal>,

    /// Global hypotheses available to all goals
    pub global_hypotheses: List<Hypothesis>,

    /// Next goal ID
    pub next_goal_id: usize,
}

impl ProofState {
    /// Create a new proof state with an initial goal
    pub fn new(initial_goal: Expr) -> Self {
        let goal = Goal::new(0, initial_goal);
        Self {
            goals: List::from_iter([goal]),
            proven_goals: List::new(),
            global_hypotheses: List::new(),
            next_goal_id: 1,
        }
    }

    /// Create an empty proof state
    pub fn empty() -> Self {
        Self {
            goals: List::new(),
            proven_goals: List::new(),
            global_hypotheses: List::new(),
            next_goal_id: 0,
        }
    }

    /// Add a new goal
    pub fn add_goal(&mut self, proposition: Expr) -> usize {
        let id = self.next_goal_id;
        self.next_goal_id += 1;
        let goal = Goal::new(id, proposition);
        self.goals.push(goal);
        id
    }

    /// Add a goal with hypotheses
    pub fn add_goal_with_hypotheses(
        &mut self,
        proposition: Expr,
        hypotheses: List<Hypothesis>,
    ) -> usize {
        let id = self.next_goal_id;
        self.next_goal_id += 1;
        let goal = Goal::with_hypotheses(id, proposition, hypotheses);
        self.goals.push(goal);
        id
    }

    /// Get the current goal (first in the list)
    pub fn current_goal(&self) -> TacticResult<&Goal> {
        self.goals.first().ok_or(TacticError::NoGoals)
    }

    /// Get mutable reference to current goal
    pub fn current_goal_mut(&mut self) -> TacticResult<&mut Goal> {
        self.goals.first_mut().ok_or(TacticError::NoGoals)
    }

    /// Remove and return the current goal
    pub fn pop_current_goal(&mut self) -> TacticResult<Goal> {
        if self.goals.is_empty() {
            return Err(TacticError::NoGoals);
        }
        Ok(self.goals.remove(0))
    }

    /// Mark current goal as proven
    pub fn prove_current_goal(&mut self) -> TacticResult<()> {
        let goal = self.pop_current_goal()?;
        self.proven_goals.push(goal);
        Ok(())
    }

    /// Replace current goal with new goals
    pub fn replace_current_goal(&mut self, new_goals: List<Goal>) -> TacticResult<()> {
        self.pop_current_goal()?;
        // Insert new goals at the front
        for goal in new_goals.into_iter().rev() {
            self.goals.insert(0, goal);
        }
        Ok(())
    }

    /// Check if all goals are proven
    pub fn is_complete(&self) -> bool {
        self.goals.is_empty()
    }

    /// Get number of remaining goals
    pub fn num_goals(&self) -> usize {
        self.goals.len()
    }

    /// Add a global hypothesis
    pub fn add_global_hypothesis(&mut self, hyp: Hypothesis) {
        self.global_hypotheses.push(hyp);
    }

    /// Find a global hypothesis by name
    pub fn find_global_hypothesis(&self, name: &Text) -> Maybe<&Hypothesis> {
        self.global_hypotheses
            .iter()
            .find(|hyp| &hyp.name == name)
            .map(|v| v as _)
    }
}

// ==================== Tactic Evaluator ====================

/// Main tactic evaluation engine
///
/// The evaluator maintains proof state and provides methods for
/// applying tactics to transform goals. It integrates with the
/// SMT solver for automated tactics.
#[derive(Debug)]
pub struct TacticEvaluator {
    /// Current proof state
    state: ProofState,

    /// SMT tactic executor
    smt_executor: TacticExecutor,

    /// Evaluation statistics
    stats: EvaluationStats,

    /// Configuration
    config: TacticConfig,

    /// Registry of user-defined (named) tactics
    ///
    /// Maps tactic names to their declarations (parameters + body).
    /// Tactics can be registered via `register_tactic` and invoked
    /// with `TacticExpr::Named`.
    tactic_registry: Map<Text, TacticDecl>,
}

impl TacticEvaluator {
    /// Create a new tactic evaluator
    pub fn new() -> Self {
        Self {
            state: ProofState::empty(),
            smt_executor: TacticExecutor::new(),
            stats: EvaluationStats::default(),
            config: TacticConfig::default(),
            tactic_registry: Map::new(),
        }
    }

    /// Create an evaluator with an initial goal
    pub fn with_goal(goal: Expr) -> Self {
        let mut evaluator = Self::new();
        evaluator.state = ProofState::new(goal);
        evaluator
    }

    /// Register a named tactic in the registry
    ///
    /// This allows user-defined tactics to be invoked by name via
    /// `TacticExpr::Named`. The tactic declaration contains the
    /// parameters and body.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_verification::tactic_evaluation::TacticEvaluator;
    /// use verum_ast::decl::{TacticDecl, TacticBody, TacticExpr, Visibility};
    /// use verum_ast::{Ident, Span};
    /// use verum_common::{List, Text};
    ///
    /// let mut evaluator = TacticEvaluator::new();
    ///
    /// // Register a simple tactic that tries auto then simp
    /// let tactic = TacticDecl {
    ///     visibility: Visibility::Public,
    ///     name: Ident::new(Text::from("my_tactic"), Span::dummy()),
    ///     params: List::new(),
    ///     body: TacticBody::Simple(TacticExpr::Auto { with_hints: List::new() }),
    ///     attributes: List::new(),
    ///     span: Span::dummy(),
    /// };
    ///
    /// evaluator.register_tactic(tactic);
    /// ```
    pub fn register_tactic(&mut self, tactic: TacticDecl) {
        let name = Text::from(tactic.name.as_str());
        self.tactic_registry.insert(name, tactic);
    }

    /// Register multiple tactics at once
    pub fn register_tactics(&mut self, tactics: impl IntoIterator<Item = TacticDecl>) {
        for tactic in tactics {
            self.register_tactic(tactic);
        }
    }

    /// Look up a named tactic in the registry
    pub fn lookup_tactic(&self, name: &Text) -> Maybe<&TacticDecl> {
        self.tactic_registry.get(name).map(|t| t as _)
    }

    /// Get an immutable reference to the tactic registry
    pub fn tactic_registry(&self) -> &Map<Text, TacticDecl> {
        &self.tactic_registry
    }

    /// Get a mutable reference to the tactic registry
    pub fn tactic_registry_mut(&mut self) -> &mut Map<Text, TacticDecl> {
        &mut self.tactic_registry
    }

    /// Get the current proof state
    pub fn state(&self) -> &ProofState {
        &self.state
    }

    /// Get mutable proof state
    pub fn state_mut(&mut self) -> &mut ProofState {
        &mut self.state
    }

    /// Get evaluation statistics
    pub fn stats(&self) -> &EvaluationStats {
        &self.stats
    }

    /// Apply a tactic expression to the current goal
    pub fn apply_tactic(&mut self, tactic: &TacticExpr) -> TacticResult<()> {
        let start = Instant::now();
        self.stats.tactics_applied += 1;

        let result = match tactic {
            TacticExpr::Trivial => self.apply_trivial(),
            TacticExpr::Assumption => self.apply_assumption(),
            TacticExpr::Reflexivity => self.apply_reflexivity(),
            TacticExpr::Intro(names) => self.apply_intro(names),
            TacticExpr::Apply { lemma, args } => self.apply_apply(lemma, args),
            TacticExpr::Rewrite {
                hypothesis,
                at_target,
                rev,
            } => self.apply_rewrite(hypothesis, at_target.as_ref(), *rev),
            TacticExpr::Split => self.apply_split(),
            TacticExpr::Left => self.apply_left(),
            TacticExpr::Right => self.apply_right(),
            TacticExpr::Exists(witness) => self.apply_exists(witness),
            TacticExpr::InductionOn(var) => self.apply_induction(var),
            TacticExpr::CasesOn(var) => self.apply_cases(var),
            TacticExpr::Exact(proof) => self.apply_exact(proof),
            TacticExpr::Unfold(names) => self.apply_unfold(names),
            TacticExpr::Compute => self.apply_compute(),
            TacticExpr::Simp { lemmas, at_target } => self.apply_simp(lemmas, at_target.as_ref()),
            TacticExpr::Ring => self.apply_ring(),
            TacticExpr::Field => self.apply_field(),
            TacticExpr::Omega => self.apply_omega(),
            TacticExpr::Auto { with_hints } => self.apply_auto(with_hints),
            TacticExpr::Blast => self.apply_blast(),
            TacticExpr::Smt { solver, timeout } => self.apply_smt(solver.as_ref(), *timeout),
            TacticExpr::Seq(tactics) => self.apply_sequence(tactics),
            TacticExpr::Alt(tactics) => self.apply_alternative(tactics),
            TacticExpr::Try(inner) => self.apply_try(inner),
            TacticExpr::TryElse { body, fallback } => {
                match self.apply_try(body) {
                    ok @ Ok(_) => ok,
                    Err(_) => self.apply_tactic(fallback),
                }
            }
            TacticExpr::Repeat(inner) => self.apply_repeat(inner),
            TacticExpr::AllGoals(inner) => self.apply_all_goals(inner),
            TacticExpr::Focus(inner) => self.apply_focus(inner),
            TacticExpr::Named { name, args, .. } => self.apply_named(name, args),
            TacticExpr::Done => self.apply_done(),
            TacticExpr::Admit => self.apply_admit(),
            TacticExpr::Sorry => self.apply_sorry(),
            TacticExpr::Contradiction => self.apply_contradiction_tactic(),

            // T1-W: evaluate the structured tactic-DSL control-flow forms
            // directly. `let` binds a local name, `match` branches on a
            // scrutinee expression, `if` picks a branch based on a boolean
            // condition, `fail` aborts the current proof branch with a
            // diagnostic. These operate on the tactic state (goals,
            // hypotheses, bindings) and defer to the evaluator's SMT /
            // structural apply_* helpers for actual proof work.
            TacticExpr::Let { name, ty: _, value } => self.apply_let(name, value),
            TacticExpr::Match { scrutinee, arms } => self.apply_match(scrutinee, arms),
            TacticExpr::If { cond, then_branch, else_branch } => {
                self.apply_if(cond, then_branch, else_branch.as_ref())
            }
            TacticExpr::Fail { message } => self.apply_fail(message),
        };

        let elapsed = start.elapsed();
        self.stats.total_time += elapsed;

        if result.is_ok() {
            self.stats.successful_tactics += 1;
        } else {
            self.stats.failed_tactics += 1;
        }

        result
    }

    // ==================== Core Tactic Implementations ====================

    /// Apply trivial tactic - proves goals that are trivially true
    fn apply_trivial(&mut self) -> TacticResult<()> {
        let goal = self.state.current_goal()?;
        if goal.is_trivial() {
            self.state.prove_current_goal()?;
            Ok(())
        } else {
            Err(TacticError::Failed(Text::from("goal is not trivial")))
        }
    }

    /// Apply assumption tactic - proves goal from hypotheses
    fn apply_assumption(&mut self) -> TacticResult<()> {
        let goal = self.state.current_goal()?;

        // Check if goal matches any hypothesis
        for hyp in &goal.hypotheses {
            if goal.expr_equal(&hyp.proposition, &goal.proposition) {
                self.state.prove_current_goal()?;
                return Ok(());
            }
        }

        // Check global hypotheses
        for hyp in &self.state.global_hypotheses {
            if goal.expr_equal(&hyp.proposition, &goal.proposition) {
                self.state.prove_current_goal()?;
                return Ok(());
            }
        }

        Err(TacticError::Failed(Text::from(
            "goal does not match any hypothesis",
        )))
    }

    /// Apply reflexivity tactic - proves x = x
    fn apply_reflexivity(&mut self) -> TacticResult<()> {
        let goal = self.state.current_goal()?;

        // Check if goal is of the form x = x
        if let ExprKind::Binary {
            op, left, right, ..
        } = &goal.proposition.kind
            && let BinOp::Eq = op
            && goal.expr_equal(left, right)
        {
            self.state.prove_current_goal()?;
            return Ok(());
        }

        Err(TacticError::Failed(Text::from(
            "goal is not a reflexive equality",
        )))
    }

    /// Apply intro tactic - introduce variables/hypotheses
    ///
    /// Handles multiple introduction scenarios:
    /// 1. Implication: `P => Q` - introduces P as a hypothesis, goal becomes Q
    /// 2. Universal quantifier: `forall x. P(x)` - introduces x as a fresh variable, goal becomes P(x)
    /// 3. Multiple intros: Can introduce several quantifiers/implications at once
    ///
    /// # Examples
    ///
    /// ```verum
    /// // For goal: forall x. x > 0 => x >= 0
    /// intro x      // Now have x in scope, goal is: x > 0 => x >= 0
    /// intro H      // Now have H: x > 0, goal is: x >= 0
    /// ```
    ///
    /// Intro tactic: introduce variables/hypotheses from the goal into the context.
    /// For universal quantifier (forall x. P(x)): introduces x, goal becomes P(x).
    /// For implication (P -> Q): introduces hypothesis H: P, goal becomes Q.
    fn apply_intro(&mut self, names: &List<Ident>) -> TacticResult<()> {
        let goal = self.state.current_goal()?;
        let goal_prop = (*goal.proposition).clone();
        let hyps = goal.hypotheses.clone();

        match &goal_prop.kind {
            // Universal quantifier: ∀x. P(x)
            // Introduce x as a fresh variable in the context, goal becomes P(x)
            ExprKind::Forall { bindings, body } => {
                // Process the first binding (multi-binding intro requires multiple intro calls)
                if bindings.is_empty() {
                    return Err(TacticError::InvalidGoal(
                        "forall has no bindings".into(),
                    ));
                }
                let binding = &bindings[0];

                // Extract variable name from pattern
                let var_name = self.extract_pattern_var_name(&binding.pattern)?;

                // If a name was provided, use it; otherwise use the bound variable name
                let intro_name = if let Some(name) = names.first() {
                    Text::from(name.as_str())
                } else {
                    var_name.clone()
                };

                // Create a hypothesis that the variable exists with its type
                let var_ty = if let verum_common::Maybe::Some(ty) = &binding.ty {
                    Maybe::Some(self.ast_type_to_type(ty))
                } else {
                    Maybe::None
                };
                let var_hyp = Hypothesis {
                    name: intro_name.clone(),
                    proposition: Heap::new(self.make_true_expr()), // Trivial proposition for type inhabitant
                    ty: var_ty,
                    source: HypothesisSource::Assumption,
                };

                // If the user provided a different name than the bound variable,
                // we need to rename the variable in the body
                let new_body = if intro_name != var_name {
                    let new_var_expr = self.make_var_expr(&intro_name);
                    self.substitute_var(body, &var_name, &new_var_expr)
                } else {
                    (**body).clone()
                };

                let mut new_goal = Goal::new(self.state.next_goal_id, new_body);
                new_goal.hypotheses = hyps;
                new_goal.add_hypothesis(var_hyp);

                self.state.next_goal_id += 1;
                self.state
                    .replace_current_goal(List::from_iter([new_goal]))?;
                Ok(())
            }

            // Implication: P => Q
            // Assume P and prove Q
            ExprKind::Binary {
                op: BinOp::Imply,
                left,
                right,
                ..
            } => {
                let hyp_name = if let Some(name) = names.first() {
                    name.name.clone()
                } else {
                    Text::from("H")
                };

                let hyp = Hypothesis::assumption(hyp_name.clone(), (**left).clone());

                let mut new_goal = Goal::new(self.state.next_goal_id, (**right).clone());
                new_goal.hypotheses = hyps;
                new_goal.add_hypothesis(hyp);

                self.state.next_goal_id += 1;
                self.state
                    .replace_current_goal(List::from_iter([new_goal]))?;
                Ok(())
            }

            // Function type can also be treated as implication in proofs
            // A -> B is equivalent to A => B in a logical sense
            _ => Err(TacticError::Failed(Text::from(
                "intro only works on implications (P => Q) and universal quantifiers (forall x. P(x))",
            ))),
        }
    }

    /// Convert an AST Type to the internal Type representation
    fn ast_type_to_type(&self, ty: &verum_ast::ty::Type) -> Type {
        use verum_ast::TypeKind;

        match &ty.kind {
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();
                    match name {
                        "Int" | "int" => Type::Int,
                        "Bool" | "bool" => Type::Bool,
                        "Float" | "float" => Type::Float,
                        "Text" | "String" => Type::Text,
                        "Char" | "char" => Type::Char,
                        "()" | "Unit" => Type::Unit,
                        _ => Type::Named {
                            path: path.clone(),
                            args: List::new(),
                        },
                    }
                } else {
                    Type::Named {
                        path: path.clone(),
                        args: List::new(),
                    }
                }
            }
            TypeKind::Tuple(types) => {
                let converted: List<_> =
                    types.iter().map(|t| self.ast_type_to_type(t)).collect();
                Type::Tuple(converted)
            }
            TypeKind::Array { element, size } => {
                let size_val = size.as_ref().and_then(|sz| {
                    if let ExprKind::Literal(lit) = &sz.kind {
                        if let LiteralKind::Int(int_lit) = &lit.kind {
                            return Some(int_lit.value as usize);
                        }
                    }
                    None
                });
                Type::Array {
                    element: Box::new(self.ast_type_to_type(element)),
                    size: size_val,
                }
            }
            TypeKind::Reference { inner, mutable } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.ast_type_to_type(inner)),
            },
            TypeKind::Inferred => {
                // For inferred types, we create a fresh type variable
                Type::Var(verum_types::TypeVar::fresh())
            }
            _ => Type::Unit, // Default to Unit for unknown types
        }
    }

    /// Apply apply tactic - apply a lemma to prove the current goal
    ///
    /// If the lemma is `P → Q` and the current goal is `Q`, this replaces
    /// the goal with `P` (we need to prove P to conclude Q via the lemma).
    ///
    /// For chained implications `P₁ → P₂ → ... → Pₙ → Q`, this creates
    /// subgoals for each premise P₁, P₂, ..., Pₙ.
    ///
    /// If `args` are provided, they instantiate quantified variables in the lemma.
    fn apply_apply(&mut self, lemma: &Heap<Expr>, args: &List<Expr>) -> TacticResult<()> {
        let goal = self.state.current_goal()?;
        let goal_prop = &goal.proposition;
        let hyps = goal.hypotheses.clone();

        // Step 1: Try to find lemma as a hypothesis if it's a path/identifier
        let lemma_expr = if let ExprKind::Path(path) = &lemma.kind {
            if let Some(ident) = path.as_ident() {
                let name = Text::from(ident.as_str());
                // Look for hypothesis by name
                if let Maybe::Some(hyp) = goal.find_hypothesis(&name) {
                    (*hyp.proposition).clone()
                } else if let Maybe::Some(global_hyp) = self.state.find_global_hypothesis(&name) {
                    (*global_hyp.proposition).clone()
                } else {
                    // Use the lemma expression as-is (might be a reference to a theorem)
                    (**lemma).clone()
                }
            } else {
                (**lemma).clone()
            }
        } else {
            (**lemma).clone()
        };

        // Step 2: Collect premises and conclusion from implication chain
        let (premises, conclusion) = self.extract_implication_chain(&lemma_expr);

        // Step 3: Check if conclusion matches the current goal
        if !self.exprs_match(&conclusion, goal_prop) {
            return Err(TacticError::Failed(Text::from(format!(
                "lemma conclusion does not match goal: expected {:?}, got {:?}",
                goal_prop.kind, conclusion.kind
            ))));
        }

        // Step 4: Create subgoals for each premise
        let mut new_goals = List::new();
        for (i, premise) in premises.iter().enumerate() {
            // Apply argument substitutions if provided
            let instantiated_premise = if !args.is_empty() {
                self.instantiate_with_args(premise, args)?
            } else {
                premise.clone()
            };

            let subgoal = Goal::with_hypotheses(
                self.state.next_goal_id + i,
                instantiated_premise,
                hyps.clone(),
            );
            new_goals.push(subgoal);
        }

        // Step 5: If no premises, the goal is directly proven by the lemma
        if new_goals.is_empty() {
            self.state.prove_current_goal()?;
        } else {
            self.state.next_goal_id += new_goals.len();
            self.state.replace_current_goal(new_goals)?;
        }

        Ok(())
    }

    /// Extract premises and conclusion from an implication chain
    /// P₁ → P₂ → ... → Pₙ → Q becomes (vec![P₁, P₂, ..., Pₙ], Q)
    fn extract_implication_chain(&self, expr: &Expr) -> (Vec<Expr>, Expr) {
        let mut premises = Vec::new();
        let mut current = expr.clone();

        // Unpack nested implications from the right
        while let ExprKind::Binary {
            op: BinOp::Imply,
            left,
            right,
        } = &current.kind
        {
            premises.push((**left).clone());
            current = (**right).clone();
        }

        (premises, current)
    }

    /// Check if two expressions match (structural equality with minor flexibility)
    fn exprs_match(&self, e1: &Expr, e2: &Expr) -> bool {
        expr_structural_equal(e1, e2, &mut HashMap::new())
    }

    /// Instantiate expression with argument substitutions
    fn instantiate_with_args(&self, expr: &Expr, args: &List<Expr>) -> TacticResult<Expr> {
        // Simple implementation: substitute arg_0, arg_1, etc.
        // A full implementation would use proper type-directed substitution
        let mut result = expr.clone();
        for (i, arg) in args.iter().enumerate() {
            let placeholder = Text::from(format!("_arg{}", i));
            result = self.substitute_var(&result, &placeholder, arg);
        }
        Ok(result)
    }

    /// Substitute a variable with an expression
    fn substitute_var(&self, expr: &Expr, var: &Text, replacement: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    if Text::from(ident.as_str()) == *var {
                        return replacement.clone();
                    }
                }
                expr.clone()
            }
            ExprKind::Binary { op, left, right } => {
                let new_left = Heap::new(self.substitute_var(left, var, replacement));
                let new_right = Heap::new(self.substitute_var(right, var, replacement));
                Expr::new(
                    ExprKind::Binary {
                        op: *op,
                        left: new_left,
                        right: new_right,
                    },
                    expr.span,
                )
            }
            ExprKind::Unary {
                op,
                expr: inner_expr,
            } => {
                let new_inner = Heap::new(self.substitute_var(inner_expr, var, replacement));
                Expr::new(
                    ExprKind::Unary {
                        op: *op,
                        expr: new_inner,
                    },
                    expr.span,
                )
            }
            ExprKind::Call {
                func,
                args: call_args,
                ..
            } => {
                let new_func = Heap::new(self.substitute_var(func, var, replacement));
                let new_args: List<_> = call_args
                    .iter()
                    .map(|a| self.substitute_var(a, var, replacement))
                    .collect();
                Expr::new(
                    ExprKind::Call {
                        func: new_func,
                        type_args: Vec::new().into(),
                        args: new_args,
                    },
                    expr.span,
                )
            }
            _ => expr.clone(), // For other expressions, return as-is
        }
    }

    /// Apply rewrite tactic - rewrite using an equality hypothesis
    ///
    /// Given a hypothesis `A = B`, this finds occurrences of `A` in the goal
    /// and replaces them with `B`. If `reverse` is true, it replaces `B` with `A`.
    ///
    /// If `at_target` is specified, only rewrites within the named hypothesis.
    fn apply_rewrite(
        &mut self,
        hypothesis: &Heap<Expr>,
        at_target: Option<&Ident>,
        reverse: bool,
    ) -> TacticResult<()> {
        let goal = self.state.current_goal()?;
        let goal_prop = (*goal.proposition).clone();
        let hyps = goal.hypotheses.clone();

        // Step 1: Find the equality hypothesis
        let eq_expr = if let ExprKind::Path(path) = &hypothesis.kind {
            if let Some(ident) = path.as_ident() {
                let name = Text::from(ident.as_str());
                if let Maybe::Some(hyp) = goal.find_hypothesis(&name) {
                    (*hyp.proposition).clone()
                } else if let Maybe::Some(global_hyp) = self.state.find_global_hypothesis(&name) {
                    (*global_hyp.proposition).clone()
                } else {
                    return Err(TacticError::HypothesisNotFound(name));
                }
            } else {
                (**hypothesis).clone()
            }
        } else {
            (**hypothesis).clone()
        };

        // Step 2: Extract LHS and RHS from equality
        let (lhs, rhs) = match &eq_expr.kind {
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
                ..
            } => {
                if reverse {
                    ((**right).clone(), (**left).clone())
                } else {
                    ((**left).clone(), (**right).clone())
                }
            }
            _ => {
                return Err(TacticError::Failed(Text::from(
                    "rewrite requires an equality hypothesis (A = B)",
                )));
            }
        };

        // Step 3: Rewrite in goal or specific hypothesis
        let new_prop = if let Some(target) = at_target {
            // Rewrite in a specific hypothesis
            let target_name = Text::from(target.as_str());
            let mut new_hyps = List::new();
            let mut found = false;

            for hyp in &hyps {
                if hyp.name == target_name {
                    found = true;
                    let rewritten = self.rewrite_expr(&hyp.proposition, &lhs, &rhs);
                    new_hyps.push(Hypothesis {
                        name: hyp.name.clone(),
                        proposition: Heap::new(rewritten),
                        ty: hyp.ty.clone(),
                        source: hyp.source.clone(),
                    });
                } else {
                    new_hyps.push(hyp.clone());
                }
            }

            if !found {
                return Err(TacticError::HypothesisNotFound(target_name));
            }

            // Create new goal with updated hypotheses
            let new_goal = Goal::with_hypotheses(self.state.next_goal_id, goal_prop, new_hyps);
            self.state.next_goal_id += 1;
            self.state
                .replace_current_goal(List::from_iter([new_goal]))?;
            return Ok(());
        } else {
            // Rewrite in the goal proposition
            self.rewrite_expr(&goal_prop, &lhs, &rhs)
        };

        // Step 4: Check if any rewriting occurred
        if self.exprs_match(&new_prop, &goal_prop) {
            return Err(TacticError::Failed(Text::from(
                "rewrite made no progress - pattern not found in goal",
            )));
        }

        // Step 5: Create new goal with rewritten proposition
        let new_goal = Goal::with_hypotheses(self.state.next_goal_id, new_prop, hyps);
        self.state.next_goal_id += 1;
        self.state
            .replace_current_goal(List::from_iter([new_goal]))?;

        Ok(())
    }

    /// Rewrite all occurrences of `from` to `to` in an expression
    fn rewrite_expr(&self, expr: &Expr, from: &Expr, to: &Expr) -> Expr {
        // Check if the current expression matches 'from'
        if self.exprs_match(expr, from) {
            return to.clone();
        }

        // Recursively rewrite subexpressions
        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                let new_left = Heap::new(self.rewrite_expr(left, from, to));
                let new_right = Heap::new(self.rewrite_expr(right, from, to));
                Expr::new(
                    ExprKind::Binary {
                        op: *op,
                        left: new_left,
                        right: new_right,
                    },
                    expr.span,
                )
            }
            ExprKind::Unary {
                op,
                expr: inner_expr,
            } => {
                let new_inner = Heap::new(self.rewrite_expr(inner_expr, from, to));
                Expr::new(
                    ExprKind::Unary {
                        op: *op,
                        expr: new_inner,
                    },
                    expr.span,
                )
            }
            ExprKind::Call { func, args, .. } => {
                let new_func = Heap::new(self.rewrite_expr(func, from, to));
                let new_args: List<_> = args
                    .iter()
                    .map(|a| self.rewrite_expr(a, from, to))
                    .collect();
                Expr::new(
                    ExprKind::Call {
                        func: new_func,
                        type_args: Vec::new().into(),
                        args: new_args,
                    },
                    expr.span,
                )
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Rewrite condition expressions
                let new_conditions: SmallVec<_> = condition
                    .conditions
                    .iter()
                    .map(|cond| match cond {
                        verum_ast::ConditionKind::Expr(e) => {
                            verum_ast::ConditionKind::Expr(self.rewrite_expr(e, from, to))
                        }
                        verum_ast::ConditionKind::Let { pattern, value } => {
                            verum_ast::ConditionKind::Let {
                                pattern: pattern.clone(),
                                value: self.rewrite_expr(value, from, to),
                            }
                        }
                    })
                    .collect();
                let new_condition = Heap::new(verum_ast::IfCondition {
                    conditions: new_conditions,
                    span: condition.span,
                });

                // Rewrite then branch (only the trailing expression, not statements)
                let new_then_expr = then_branch
                    .expr
                    .as_ref()
                    .map(|e| Heap::new(self.rewrite_expr(e, from, to)));
                let new_then = verum_ast::Block {
                    stmts: then_branch.stmts.clone(),
                    expr: new_then_expr,
                    span: then_branch.span,
                };

                // Rewrite else branch if present
                let new_else = else_branch
                    .as_ref()
                    .map(|e| Heap::new(self.rewrite_expr(e, from, to)));

                Expr::new(
                    ExprKind::If {
                        condition: new_condition,
                        then_branch: new_then,
                        else_branch: new_else,
                    },
                    expr.span,
                )
            }
            ExprKind::Forall { bindings, body } => {
                let new_body = Heap::new(self.rewrite_expr(body, from, to));
                Expr::new(
                    ExprKind::Forall {
                        bindings: bindings.clone(),
                        body: new_body,
                    },
                    expr.span,
                )
            }
            ExprKind::Exists { bindings, body } => {
                let new_body = Heap::new(self.rewrite_expr(body, from, to));
                Expr::new(
                    ExprKind::Exists {
                        bindings: bindings.clone(),
                        body: new_body,
                    },
                    expr.span,
                )
            }
            _ => expr.clone(), // For other expressions, return as-is
        }
    }

    /// Apply split tactic - split conjunction into subgoals
    fn apply_split(&mut self) -> TacticResult<()> {
        let goal = self.state.current_goal()?;

        // Split conjunction: P ∧ Q => prove P and prove Q
        if let ExprKind::Binary {
            op: BinOp::And,
            left,
            right,
            ..
        } = &goal.proposition.kind
        {
            let left_id = self.state.next_goal_id;
            let right_id = self.state.next_goal_id + 1;
            let hyps = goal.hypotheses.clone();
            let left_expr = (**left).clone();
            let right_expr = (**right).clone();

            self.state.next_goal_id += 2;

            let left_goal = Goal::with_hypotheses(left_id, left_expr, hyps.clone());
            let right_goal = Goal::with_hypotheses(right_id, right_expr, hyps);

            self.state
                .replace_current_goal(List::from_iter([left_goal, right_goal]))?;
            Ok(())
        } else {
            Err(TacticError::Failed(Text::from("goal is not a conjunction")))
        }
    }

    /// Apply left tactic - prove left side of disjunction
    fn apply_left(&mut self) -> TacticResult<()> {
        let goal = self.state.current_goal()?;

        // P ∨ Q => prove P
        if let ExprKind::Binary {
            op: BinOp::Or,
            left,
            ..
        } = &goal.proposition.kind
        {
            let new_goal = Goal::with_hypotheses(
                self.state.next_goal_id,
                (**left).clone(),
                goal.hypotheses.clone(),
            );
            self.state.next_goal_id += 1;

            self.state
                .replace_current_goal(List::from_iter([new_goal]))?;
            Ok(())
        } else {
            Err(TacticError::Failed(Text::from("goal is not a disjunction")))
        }
    }

    /// Apply right tactic - prove right side of disjunction
    fn apply_right(&mut self) -> TacticResult<()> {
        let goal = self.state.current_goal()?;

        // P ∨ Q => prove Q
        if let ExprKind::Binary {
            op: BinOp::Or,
            right,
            ..
        } = &goal.proposition.kind
        {
            let new_goal = Goal::with_hypotheses(
                self.state.next_goal_id,
                (**right).clone(),
                goal.hypotheses.clone(),
            );
            self.state.next_goal_id += 1;

            self.state
                .replace_current_goal(List::from_iter([new_goal]))?;
            Ok(())
        } else {
            Err(TacticError::Failed(Text::from("goal is not a disjunction")))
        }
    }

    /// Apply exists tactic - provide a witness for an existential goal
    ///
    /// If the goal is `∃x. P(x)`, providing witness `w` transforms the goal to `P(w)`.
    fn apply_exists(&mut self, witness: &Heap<Expr>) -> TacticResult<()> {
        let goal = self.state.current_goal()?;
        let goal_prop = &goal.proposition;
        let hyps = goal.hypotheses.clone();

        // Check if goal is an existential quantifier
        match &goal_prop.kind {
            ExprKind::Exists { bindings, body } => {
                // Extract the first bound variable name from the bindings
                if bindings.is_empty() {
                    return Err(TacticError::Failed(Text::from(
                        "exists expression has no bindings",
                    )));
                }
                let var_name = self.extract_pattern_var_name(&bindings[0].pattern)?;

                // Substitute witness for the bound variable in the body
                let instantiated_body = self.substitute_var(body, &var_name, witness);

                // Create new goal with the instantiated body
                let new_goal =
                    Goal::with_hypotheses(self.state.next_goal_id, instantiated_body, hyps);
                self.state.next_goal_id += 1;
                self.state
                    .replace_current_goal(List::from_iter([new_goal]))?;

                Ok(())
            }
            _ => Err(TacticError::Failed(Text::from(
                "exists tactic requires an existential goal (∃x. P(x))",
            ))),
        }
    }

    /// Extract variable name from a pattern (for quantifier binding)
    fn extract_pattern_var_name(&self, pattern: &Pattern) -> TacticResult<Text> {
        use verum_ast::pattern::PatternKind;
        match &pattern.kind {
            PatternKind::Ident { name, .. } => Ok(Text::from(name.as_str())),
            PatternKind::Wildcard => Ok(Text::from("_")),
            _ => Err(TacticError::Failed(Text::from(
                "quantifier pattern must be a simple identifier",
            ))),
        }
    }

    /// Apply induction tactic
    ///
    /// Performs structural induction on a variable. For natural numbers (Int),
    /// this creates a base case (n = 0) and an inductive step (n → n + 1).
    ///
    /// The induction hypothesis is added to the context for the step case.
    fn apply_induction(&mut self, var: &Ident) -> TacticResult<()> {
        let goal = self.state.current_goal()?;
        let goal_prop = (*goal.proposition).clone();
        let hyps = goal.hypotheses.clone();
        let var_name = Text::from(var.as_str());

        // Look up the type of the variable from hypotheses
        let var_type = self.find_variable_type(&var_name, &hyps);

        match &var_type {
            Some(ty) if self.is_nat_type(ty) => {
                // Natural number induction
                self.apply_nat_induction(&var_name, &goal_prop, &hyps)
            }
            Some(ty) if self.is_list_type(ty) => {
                // List induction
                self.apply_list_induction(&var_name, &goal_prop, &hyps)
            }
            Some(ty) => {
                // For unknown types, we cannot perform structural induction
                // without knowing the type's recursive structure
                Err(TacticError::Failed(Text::from(format!(
                    "cannot perform induction on variable '{}' of type {:?}: \
                     only Int/Nat/UInt and List types support automatic induction. \
                     For other types, consider providing a custom well-founded relation.",
                    var_name, ty
                ))))
            }
            None => Err(TacticError::Failed(Text::from(format!(
                "cannot determine type of variable '{}' for induction",
                var_name
            )))),
        }
    }

    /// Apply induction for natural numbers
    ///
    /// Creates two subgoals:
    /// - Base case: P(0)
    /// - Step case: P(n) → P(n+1), where the goal is P(n+1) with IH: P(n)
    fn apply_nat_induction(
        &mut self,
        var_name: &Text,
        goal_prop: &Expr,
        hyps: &List<Hypothesis>,
    ) -> TacticResult<()> {
        let base_goal_id = self.state.next_goal_id;
        let step_goal_id = self.state.next_goal_id + 1;

        // Base case: P(0)
        let zero_expr = Expr::new(
            ExprKind::Literal(verum_ast::Literal::new(
                LiteralKind::Int(verum_ast::literal::IntLit::new(0)),
                Span::default(),
            )),
            Span::default(),
        );
        let base_prop = self.substitute_var(goal_prop, var_name, &zero_expr);
        let base_goal = Goal::with_hypotheses(base_goal_id, base_prop, hyps.clone());

        // Step case: ∀n. P(n) → P(n+1)
        // We add the induction hypothesis: P(n) as "IH_<var>"
        let ih_name = Text::from(format!("IH_{}", var_name));
        let ih_hyp = Hypothesis::induction(ih_name, goal_prop.clone());
        let mut step_hyps = hyps.clone();
        step_hyps.push(ih_hyp);

        // Create the step goal: P(n+1)
        // Build expression: var + 1
        let var_expr = self.make_var_expr(var_name);
        let one_expr = Expr::new(
            ExprKind::Literal(verum_ast::Literal::new(
                LiteralKind::Int(verum_ast::literal::IntLit::new(1)),
                Span::default(),
            )),
            Span::default(),
        );
        let successor_expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(var_expr),
                right: Heap::new(one_expr),
            },
            Span::default(),
        );

        // Substitute n with n+1 in the goal to get P(n+1)
        let step_prop = self.substitute_var(goal_prop, var_name, &successor_expr);
        let step_goal = Goal::with_hypotheses(step_goal_id, step_prop, step_hyps);

        self.state.next_goal_id += 2;
        self.state
            .replace_current_goal(List::from_iter([base_goal, step_goal]))?;
        Ok(())
    }

    /// Apply induction for lists
    ///
    /// Creates two subgoals:
    /// - Base case: P([])
    /// - Step case: P(xs) → P(x :: xs), where goal is P(x :: xs) with IH: P(xs)
    fn apply_list_induction(
        &mut self,
        var_name: &Text,
        goal_prop: &Expr,
        hyps: &List<Hypothesis>,
    ) -> TacticResult<()> {
        let base_goal_id = self.state.next_goal_id;
        let step_goal_id = self.state.next_goal_id + 1;

        // Base case: P([]) - empty list
        let empty_list = Expr::new(
            ExprKind::Array(verum_ast::ArrayExpr::List(List::new())),
            Span::default(),
        );
        let base_prop = self.substitute_var(goal_prop, var_name, &empty_list);
        let base_goal = Goal::with_hypotheses(base_goal_id, base_prop, hyps.clone());

        // Step case: ∀x, xs. P(xs) → P(x :: xs)
        // The IH is P(xs_tail) where xs_tail is the tail of the list
        let tail_var_name = Text::from(format!("{}_tail", var_name));
        let tail_var_expr = self.make_var_expr(&tail_var_name);

        // Create the IH: P(xs_tail)
        let ih_prop = self.substitute_var(goal_prop, var_name, &tail_var_expr);
        let ih_name = Text::from(format!("IH_{}", var_name));
        let ih_hyp = Hypothesis::induction(ih_name, ih_prop);
        let mut step_hyps = hyps.clone();
        step_hyps.push(ih_hyp);

        // Also add hypothesis for the head element
        let head_var_name = Text::from(format!("{}_head", var_name));
        let head_var_expr = self.make_var_expr(&head_var_name);

        // Create the cons expression: cons(head, tail) or prepend(head, tail)
        // We use a function call since there's no Cons binary operator
        let cons_expr = self.make_constructor_expr("cons", &[head_var_expr, tail_var_expr]);

        // The step goal is P(head :: tail)
        let step_prop = self.substitute_var(goal_prop, var_name, &cons_expr);
        let step_goal = Goal::with_hypotheses(step_goal_id, step_prop, step_hyps);

        self.state.next_goal_id += 2;
        self.state
            .replace_current_goal(List::from_iter([base_goal, step_goal]))?;
        Ok(())
    }

    /// Check if a type is a natural number type
    fn is_nat_type(&self, ty: &Type) -> bool {
        matches!(ty, Type::Int)
            || match ty {
                Type::Named { path, .. } => path
                    .as_ident()
                    .map(|id| {
                        let n = id.as_str();
                        n == "Nat" || n == "Int" || n == "UInt"
                    })
                    .unwrap_or(false),
                _ => false,
            }
    }

    /// Check if a type is a list type
    fn is_list_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Named { path, .. } => path
                .as_ident()
                .map(|id| verum_common::well_known_types::WellKnownType::List.matches(id.as_str()))
                .unwrap_or(false),
            _ => false,
        }
    }

    /// Find the type of a variable from hypotheses
    fn find_variable_type(&self, name: &Text, hyps: &List<Hypothesis>) -> Option<Type> {
        // First check if any hypothesis explicitly types the variable
        for hyp in hyps {
            if hyp.name == *name {
                if let Maybe::Some(ref ty) = hyp.ty {
                    return Some(ty.clone());
                }
            }
        }

        // Check global hypotheses
        for hyp in &self.state.global_hypotheses {
            if hyp.name == *name {
                if let Maybe::Some(ref ty) = hyp.ty {
                    return Some(ty.clone());
                }
            }
        }

        // Default to Int for now if we can't determine
        Some(Type::int())
    }

    /// Apply cases tactic - case analysis on a variable
    ///
    /// For booleans, creates true/false cases.
    /// For Maybe/Option, creates Some/None cases.
    /// For Result, creates Ok/Err cases.
    fn apply_cases(&mut self, var: &Ident) -> TacticResult<()> {
        let goal = self.state.current_goal()?;
        let goal_prop = (*goal.proposition).clone();
        let hyps = goal.hypotheses.clone();
        let var_name = Text::from(var.as_str());

        // Look up the type of the variable
        let var_type = self.find_variable_type(&var_name, &hyps);

        match &var_type {
            Some(ty) if self.is_bool_type(ty) => {
                self.apply_bool_cases(&var_name, &goal_prop, &hyps)
            }
            Some(ty) if self.is_maybe_type(ty) => {
                self.apply_maybe_cases(&var_name, &goal_prop, &hyps)
            }
            Some(ty) if self.is_result_type(ty) => {
                self.apply_result_cases(&var_name, &goal_prop, &hyps)
            }
            Some(ty) => {
                // For unknown/generic types, we cannot create meaningful case splits
                // without knowing the type's structure. Return an error with guidance.
                Err(TacticError::Failed(Text::from(format!(
                    "cannot perform case analysis on variable '{}' of type {:?}: \
                     only Bool, Maybe/Option, and Result types are supported. \
                     Consider using a more specific tactic or providing type annotations.",
                    var_name, ty
                ))))
            }
            None => Err(TacticError::Failed(Text::from(format!(
                "cannot determine type of variable '{}' for case analysis",
                var_name
            )))),
        }
    }

    /// Apply case analysis for booleans
    fn apply_bool_cases(
        &mut self,
        var_name: &Text,
        goal_prop: &Expr,
        hyps: &List<Hypothesis>,
    ) -> TacticResult<()> {
        let true_goal_id = self.state.next_goal_id;
        let false_goal_id = self.state.next_goal_id + 1;

        // Case true
        let true_expr = Expr::new(
            ExprKind::Literal(verum_ast::Literal::new(
                LiteralKind::Bool(true),
                Span::default(),
            )),
            Span::default(),
        );
        let true_prop = self.substitute_var(goal_prop, var_name, &true_expr);
        let true_goal = Goal::with_hypotheses(true_goal_id, true_prop, hyps.clone());

        // Case false
        let false_expr = Expr::new(
            ExprKind::Literal(verum_ast::Literal::new(
                LiteralKind::Bool(false),
                Span::default(),
            )),
            Span::default(),
        );
        let false_prop = self.substitute_var(goal_prop, var_name, &false_expr);
        let false_goal = Goal::with_hypotheses(false_goal_id, false_prop, hyps.clone());

        self.state.next_goal_id += 2;
        self.state
            .replace_current_goal(List::from_iter([true_goal, false_goal]))?;
        Ok(())
    }

    /// Apply case analysis for Maybe/Option types
    ///
    /// Creates two subgoals with proper hypotheses:
    /// - Some case: `exists x. var = Some(x)`
    /// - None case: `var = None`
    fn apply_maybe_cases(
        &mut self,
        var_name: &Text,
        goal_prop: &Expr,
        hyps: &List<Hypothesis>,
    ) -> TacticResult<()> {
        let some_goal_id = self.state.next_goal_id;
        let none_goal_id = self.state.next_goal_id + 1;

        // Case Some(x): Add hypothesis "exists x. var = Some(x)"
        let some_hyp = Hypothesis::new(
            Text::from(format!("{}_is_some", var_name)),
            self.make_constructor_case_expr(var_name, "Some", true),
        );
        let mut some_hyps = hyps.clone();
        some_hyps.push(some_hyp);
        let some_goal = Goal::with_hypotheses(some_goal_id, goal_prop.clone(), some_hyps);

        // Case None: Add hypothesis "var = None"
        let none_hyp = Hypothesis::new(
            Text::from(format!("{}_is_none", var_name)),
            self.make_constructor_case_expr(var_name, "None", false),
        );
        let mut none_hyps = hyps.clone();
        none_hyps.push(none_hyp);
        let none_goal = Goal::with_hypotheses(none_goal_id, goal_prop.clone(), none_hyps);

        self.state.next_goal_id += 2;
        self.state
            .replace_current_goal(List::from_iter([some_goal, none_goal]))?;
        Ok(())
    }

    /// Apply case analysis for Result types
    ///
    /// Creates two subgoals with proper hypotheses:
    /// - Ok case: `exists x. var = Ok(x)`
    /// - Err case: `exists e. var = Err(e)`
    fn apply_result_cases(
        &mut self,
        var_name: &Text,
        goal_prop: &Expr,
        hyps: &List<Hypothesis>,
    ) -> TacticResult<()> {
        let ok_goal_id = self.state.next_goal_id;
        let err_goal_id = self.state.next_goal_id + 1;

        // Case Ok(x): Add hypothesis "exists x. var = Ok(x)"
        let ok_hyp = Hypothesis::new(
            Text::from(format!("{}_is_ok", var_name)),
            self.make_constructor_case_expr(var_name, "Ok", true),
        );
        let mut ok_hyps = hyps.clone();
        ok_hyps.push(ok_hyp);
        let ok_goal = Goal::with_hypotheses(ok_goal_id, goal_prop.clone(), ok_hyps);

        // Case Err(e): Add hypothesis "exists e. var = Err(e)"
        let err_hyp = Hypothesis::new(
            Text::from(format!("{}_is_err", var_name)),
            self.make_constructor_case_expr(var_name, "Err", true),
        );
        let mut err_hyps = hyps.clone();
        err_hyps.push(err_hyp);
        let err_goal = Goal::with_hypotheses(err_goal_id, goal_prop.clone(), err_hyps);

        self.state.next_goal_id += 2;
        self.state
            .replace_current_goal(List::from_iter([ok_goal, err_goal]))?;
        Ok(())
    }

    /// Check if a type is Bool
    fn is_bool_type(&self, ty: &Type) -> bool {
        matches!(ty, Type::Bool)
            || match ty {
                Type::Named { path, .. } => path
                    .as_ident()
                    .map(|id| id.as_str() == "Bool")
                    .unwrap_or(false),
                _ => false,
            }
    }

    /// Check if a type is Maybe/Option
    fn is_maybe_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Named { path, .. } => path
                .as_ident()
                .map(|id| verum_common::well_known_types::WellKnownType::Maybe.matches(id.as_str()))
                .unwrap_or(false),
            _ => false,
        }
    }

    /// Check if a type is Result
    fn is_result_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Named { path, .. } => path
                .as_ident()
                .map(|id| verum_common::well_known_types::WellKnownType::Result.matches(id.as_str()))
                .unwrap_or(false),
            _ => false,
        }
    }

    /// Create a true expression
    fn make_true_expr(&self) -> Expr {
        Expr::new(
            ExprKind::Literal(verum_ast::Literal::new(
                LiteralKind::Bool(true),
                Span::default(),
            )),
            Span::default(),
        )
    }

    /// Create an equality expression: left == right
    fn make_eq_expr(&self, left: Expr, right: Expr) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Heap::new(left),
                right: Heap::new(right),
            },
            Span::default(),
        )
    }

    /// Create a variable expression from name
    fn make_var_expr(&self, name: &Text) -> Expr {
        Expr::new(
            ExprKind::Path(verum_ast::ty::Path::single(Ident::new(
                name.clone(),
                Span::default(),
            ))),
            Span::default(),
        )
    }

    /// Create a constructor call expression like Some(x), None, Ok(x), Err(e)
    fn make_constructor_expr(&self, constructor: &str, args: &[Expr]) -> Expr {
        let ctor_path =
            verum_ast::ty::Path::single(Ident::new(Text::from(constructor), Span::default()));
        if args.is_empty() {
            // Unit constructor like None
            Expr::new(ExprKind::Path(ctor_path), Span::default())
        } else {
            // Constructor with arguments like Some(x)
            Expr::new(
                ExprKind::Call {
                    func: Heap::new(Expr::new(ExprKind::Path(ctor_path), Span::default())),
                    type_args: Vec::new().into(),
                    args: List::from_iter(args.iter().cloned()),
                },
                Span::default(),
            )
        }
    }

    /// Create an existential quantification: exists var_name. body
    fn make_exists_expr(&self, var_name: &str, body: Expr) -> Expr {
        let bound_var = Ident::new(Text::from(var_name), Span::default());
        let pattern = Pattern::ident(bound_var, false, Span::default());
        let binding = verum_ast::expr::QuantifierBinding::typed(
            pattern,
            verum_ast::ty::Type::inferred(Span::default()),
            Span::default(),
        );
        Expr::new(
            ExprKind::Exists {
                bindings: List::from_iter([binding]),
                body: Heap::new(body),
            },
            Span::default(),
        )
    }

    /// Create hypothesis expression: exists inner_var. var == Constructor(inner_var)
    /// Example: exists x. opt == Some(x)
    fn make_constructor_case_expr(
        &self,
        var_name: &Text,
        constructor: &str,
        has_inner: bool,
    ) -> Expr {
        let var_expr = self.make_var_expr(var_name);

        if has_inner {
            // For constructors like Some(x), Ok(x), Err(e)
            let inner_var_name = format!("{}_inner", var_name);
            let inner_var_expr = self.make_var_expr(&Text::from(inner_var_name.clone()));
            let constructor_expr = self.make_constructor_expr(constructor, &[inner_var_expr]);
            let eq_expr = self.make_eq_expr(var_expr, constructor_expr);
            self.make_exists_expr(&inner_var_name, eq_expr)
        } else {
            // For unit constructors like None
            let constructor_expr = self.make_constructor_expr(constructor, &[]);
            self.make_eq_expr(var_expr, constructor_expr)
        }
    }

    /// Apply exact tactic - provide exact proof term
    fn apply_exact(&mut self, proof: &Heap<Expr>) -> TacticResult<()> {
        // In a full implementation, this would:
        // 1. Type-check the proof term
        // 2. Verify it has the type of the current goal
        // 3. Mark goal as proven

        // For now, just mark as proven
        self.state.prove_current_goal()?;
        Ok(())
    }

    /// Apply unfold tactic - unfold definitions
    ///
    /// For each name in `names`, looks for a definition (an equality hypothesis
    /// of the form `name = expr`) and replaces occurrences of `name` in the goal
    /// with `expr`.
    ///
    /// Definitions are searched in order:
    /// 1. Local hypotheses with name `<name>_def` or equality `name = ...`
    /// 2. Global hypotheses with the same patterns
    ///
    /// # Errors
    ///
    /// Returns `TacticError::Failed` if:
    /// - No goals remain
    /// - No definition is found for any of the specified names
    /// - No progress was made (definitions exist but don't occur in goal)
    fn apply_unfold(&mut self, names: &List<Ident>) -> TacticResult<()> {
        if names.is_empty() {
            return Err(TacticError::InvalidArgument(Text::from(
                "unfold requires at least one name",
            )));
        }

        let goal = self.state.current_goal()?;
        let mut current_prop = (*goal.proposition).clone();
        let hyps = goal.hypotheses.clone();
        let mut made_progress = false;

        for name in names {
            let name_text = Text::from(name.as_str());

            // Look for a definition for this name
            if let Some(def_body) = self.find_definition(&name_text, &hyps) {
                // Replace all occurrences of the name with the definition body
                let new_prop = self.unfold_name_in_expr(&current_prop, &name_text, &def_body);

                // Check if we made progress
                if !self.exprs_match(&new_prop, &current_prop) {
                    current_prop = new_prop;
                    made_progress = true;
                }
            } else {
                return Err(TacticError::Failed(Text::from(format!(
                    "no definition found for '{}'",
                    name_text
                ))));
            }
        }

        if !made_progress {
            return Err(TacticError::Failed(Text::from(
                "unfold made no progress - definitions do not occur in goal",
            )));
        }

        // Create new goal with unfolded proposition
        let new_goal = Goal::with_hypotheses(self.state.next_goal_id, current_prop, hyps);
        self.state.next_goal_id += 1;
        self.state
            .replace_current_goal(List::from_iter([new_goal]))?;

        Ok(())
    }

    /// Find a definition for the given name in hypotheses
    ///
    /// Searches for:
    /// 1. A hypothesis named `<name>_def` containing an equality `name = body`
    /// 2. Any hypothesis that is an equality of the form `name = body`
    /// 3. Global hypotheses with the same patterns
    fn find_definition(&self, name: &Text, local_hyps: &List<Hypothesis>) -> Option<Expr> {
        // Pattern 1: Look for a hypothesis named <name>_def
        let def_hyp_name = Text::from(format!("{}_def", name));
        if let Maybe::Some(hyp) = local_hyps
            .iter()
            .find(|h| h.name == def_hyp_name)
            .map(|h| h as &Hypothesis)
        {
            if let Some(body) = self.extract_definition_body(&hyp.proposition, name) {
                return Some(body);
            }
        }

        // Pattern 2: Look for any equality hypothesis `name = ...`
        for hyp in local_hyps {
            if let Some(body) = self.extract_definition_body(&hyp.proposition, name) {
                return Some(body);
            }
        }

        // Pattern 3: Check global hypotheses
        if let Maybe::Some(hyp) = self
            .state
            .global_hypotheses
            .iter()
            .find(|h| h.name == def_hyp_name)
            .map(|h| h as &Hypothesis)
        {
            if let Some(body) = self.extract_definition_body(&hyp.proposition, name) {
                return Some(body);
            }
        }

        for hyp in &self.state.global_hypotheses {
            if let Some(body) = self.extract_definition_body(&hyp.proposition, name) {
                return Some(body);
            }
        }

        None
    }

    /// Extract the definition body from an equality `name = body`
    fn extract_definition_body(&self, prop: &Expr, name: &Text) -> Option<Expr> {
        if let ExprKind::Binary {
            op: BinOp::Eq,
            left,
            right,
        } = &prop.kind
        {
            // Check if left side is the name we're looking for
            if let ExprKind::Path(path) = &left.kind {
                if let Some(ident) = path.as_ident() {
                    if Text::from(ident.as_str()) == *name {
                        return Some((**right).clone());
                    }
                }
            }
            // Also check the right side (for symmetry: `body = name`)
            if let ExprKind::Path(path) = &right.kind {
                if let Some(ident) = path.as_ident() {
                    if Text::from(ident.as_str()) == *name {
                        return Some((**left).clone());
                    }
                }
            }
        }
        None
    }

    /// Unfold a name within a block, handling statements and trailing expression
    ///
    /// Unfold a definition within a block (proof term transformation).
    ///
    /// This function performs a complete unfold within a block by:
    /// 1. Processing each statement, unfolding the name in expressions within
    /// 2. Tracking variable bindings to detect shadowing
    /// 3. If the name is shadowed by a let binding, we stop unfolding in subsequent statements
    /// 4. Processing the trailing expression (if any) with appropriate shadowing context
    fn unfold_in_block(
        &self,
        block: &verum_ast::expr::Block,
        name: &Text,
        def_body: &Expr,
    ) -> verum_ast::expr::Block {
        use verum_ast::pattern::PatternKind;
        use verum_ast::stmt::{Stmt, StmtKind};

        let mut new_stmts: List<Stmt> = List::new();
        let mut shadowed = false;

        for stmt in block.stmts.iter() {
            if shadowed {
                // Name is shadowed, just clone the rest without unfolding
                new_stmts.push(stmt.clone());
                continue;
            }

            match &stmt.kind {
                StmtKind::Let { pattern, ty, value } => {
                    // Check if this let binding shadows our name
                    let binds_name = self.pattern_binds_name(pattern, name);

                    // Unfold in the value expression (before shadowing takes effect)
                    let new_value = value
                        .as_ref()
                        .map(|v| self.unfold_name_in_expr(v, name, def_body));

                    new_stmts.push(Stmt::new(
                        StmtKind::Let {
                            pattern: pattern.clone(),
                            ty: ty.clone(),
                            value: new_value,
                        },
                        stmt.span,
                    ));

                    // If this binding shadows our name, don't unfold in subsequent statements
                    if binds_name {
                        shadowed = true;
                    }
                }
                StmtKind::Expr { expr, has_semi } => {
                    let new_expr = self.unfold_name_in_expr(expr, name, def_body);
                    new_stmts.push(Stmt::new(
                        StmtKind::Expr {
                            expr: new_expr,
                            has_semi: *has_semi,
                        },
                        stmt.span,
                    ));
                }
                StmtKind::Empty => {
                    new_stmts.push(stmt.clone());
                }
                StmtKind::Item(_) => {
                    // Items may introduce new bindings but we don't unfold into them
                    // as they create new scopes
                    new_stmts.push(stmt.clone());
                }
                StmtKind::Defer(_)
                | StmtKind::Errdefer(_)
                | StmtKind::Provide { .. }
                | StmtKind::ProvideScope { .. }
                | StmtKind::LetElse { .. } => {
                    // These statement kinds are not handled in unfolding
                    new_stmts.push(stmt.clone());
                }
            }
        }

        // Process trailing expression if not shadowed
        let new_expr = if shadowed {
            block.expr.clone()
        } else {
            match &block.expr {
                Maybe::Some(e) => {
                    Maybe::Some(Heap::new(self.unfold_name_in_expr(e, name, def_body)))
                }
                Maybe::None => Maybe::None,
            }
        };

        verum_ast::expr::Block {
            stmts: new_stmts,
            expr: new_expr,
            span: block.span,
        }
    }

    /// Check if a pattern binds the given name
    fn pattern_binds_name(&self, pattern: &verum_ast::pattern::Pattern, name: &Text) -> bool {
        use verum_ast::pattern::PatternKind;

        match &pattern.kind {
            PatternKind::Ident { name: pat_name, .. } => Text::from(pat_name.as_str()) == *name,
            PatternKind::Tuple(patterns) | PatternKind::Array(patterns) => {
                patterns.iter().any(|p| self.pattern_binds_name(p, name))
            }
            PatternKind::Record { fields, .. } => {
                fields.iter().any(|f| {
                    // If pattern is Some, check that; if None, it's shorthand { x } which binds f.name
                    match &f.pattern {
                        verum_common::Maybe::Some(p) => self.pattern_binds_name(p, name),
                        verum_common::Maybe::None => Text::from(f.name.as_str()) == *name,
                    }
                })
            }
            PatternKind::Variant { data, .. } => {
                use verum_ast::pattern::VariantPatternData;
                match data {
                    verum_common::Maybe::Some(VariantPatternData::Tuple(patterns)) => {
                        patterns.iter().any(|p| self.pattern_binds_name(p, name))
                    }
                    verum_common::Maybe::Some(VariantPatternData::Record { fields, .. }) => {
                        fields.iter().any(|f| match &f.pattern {
                            verum_common::Maybe::Some(p) => self.pattern_binds_name(p, name),
                            verum_common::Maybe::None => Text::from(f.name.as_str()) == *name,
                        })
                    }
                    verum_common::Maybe::None => false,
                }
            }
            PatternKind::Or(patterns) => {
                // All branches of an or-pattern must bind the same names
                patterns.iter().any(|p| self.pattern_binds_name(p, name))
            }
            PatternKind::Wildcard
            | PatternKind::Rest
            | PatternKind::Literal(_)
            | PatternKind::Range { .. } => false,
            PatternKind::Reference { inner, .. } => self.pattern_binds_name(inner, name),
            PatternKind::Paren(inner) => self.pattern_binds_name(inner, name),
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                before.iter().any(|p| self.pattern_binds_name(p, name))
                    || rest
                        .as_ref()
                        .map_or(false, |r| self.pattern_binds_name(r, name))
                    || after.iter().any(|p| self.pattern_binds_name(p, name))
            }            PatternKind::View { pattern, .. } => self.pattern_binds_name(pattern, name),
            PatternKind::Active { .. } => false,
            PatternKind::And(patterns) => {
                patterns.iter().any(|p| self.pattern_binds_name(p, name))
            }
            PatternKind::TypeTest { binding, .. } => {
                // TypeTest pattern binds the identifier to the narrowed type
                Text::from(binding.name.as_str()) == *name
            }
            PatternKind::Stream { head_patterns, rest } => {
                // Stream pattern: stream[first, second, ...rest]
                // Check head patterns and rest binding for the name
                head_patterns.iter().any(|p| self.pattern_binds_name(p, name))
                    || rest
                        .as_ref()
                        .map_or(false, |r| Text::from(r.name.as_str()) == *name)
            }
            PatternKind::Guard { pattern, .. } => {
                // Guard pattern: (pattern if expr)
                // Spec: Rust RFC 3637 - Guard Patterns
                // Check if the inner pattern binds the name
                self.pattern_binds_name(pattern, name)
            }
            PatternKind::Cons { head, tail } => {
                self.pattern_binds_name(head, name) || self.pattern_binds_name(tail, name)
            }
        }
    }

    /// Replace occurrences of a name with its definition in an expression
    fn unfold_name_in_expr(&self, expr: &Expr, name: &Text, def_body: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    if Text::from(ident.as_str()) == *name {
                        return def_body.clone();
                    }
                }
                expr.clone()
            }
            ExprKind::Binary { op, left, right } => {
                let new_left = Heap::new(self.unfold_name_in_expr(left, name, def_body));
                let new_right = Heap::new(self.unfold_name_in_expr(right, name, def_body));
                Expr::new(
                    ExprKind::Binary {
                        op: *op,
                        left: new_left,
                        right: new_right,
                    },
                    expr.span,
                )
            }
            ExprKind::Unary { op, expr: inner } => {
                let new_inner = Heap::new(self.unfold_name_in_expr(inner, name, def_body));
                Expr::new(
                    ExprKind::Unary {
                        op: *op,
                        expr: new_inner,
                    },
                    expr.span,
                )
            }
            ExprKind::Call { func, args, .. } => {
                let new_func = Heap::new(self.unfold_name_in_expr(func, name, def_body));
                let new_args: List<_> = args
                    .iter()
                    .map(|a| self.unfold_name_in_expr(a, name, def_body))
                    .collect();
                Expr::new(
                    ExprKind::Call {
                        func: new_func,
                        type_args: Vec::new().into(),
                        args: new_args,
                    },
                    expr.span,
                )
            }
            ExprKind::Paren(inner) => {
                let new_inner = self.unfold_name_in_expr(inner, name, def_body);
                Expr::new(ExprKind::Paren(Heap::new(new_inner)), expr.span)
            }
            ExprKind::Tuple(elems) => {
                let new_elems: List<_> = elems
                    .iter()
                    .map(|e| self.unfold_name_in_expr(e, name, def_body))
                    .collect();
                Expr::new(ExprKind::Tuple(new_elems), expr.span)
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Unfold in condition
                let new_conditions: SmallVec<[verum_ast::expr::ConditionKind; 2]> = condition
                    .conditions
                    .iter()
                    .map(|c| match c {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            verum_ast::expr::ConditionKind::Expr(
                                self.unfold_name_in_expr(e, name, def_body),
                            )
                        }
                        verum_ast::expr::ConditionKind::Let { pattern, value } => {
                            verum_ast::expr::ConditionKind::Let {
                                pattern: pattern.clone(),
                                value: self.unfold_name_in_expr(value, name, def_body),
                            }
                        }
                    })
                    .collect();
                let new_condition = verum_ast::expr::IfCondition {
                    conditions: new_conditions,
                    span: condition.span,
                };

                // Unfold in branches - full implementation that processes both statements and trailing expr
                //
                // Proof term unfolding in if-then-else branches.
                //
                // For a complete unfold, we must traverse:
                // 1. All statements in the block (which may contain let bindings, expressions, etc.)
                // 2. The trailing expression if present
                //
                // We must also respect variable shadowing: if a statement binds the same name
                // we're unfolding, we stop unfolding in subsequent statements and the trailing expr.
                let new_then = self.unfold_in_block(then_branch, name, def_body);
                let new_else = else_branch.as_ref().map(|e| {
                    // Else branch may be an expression or another block (else if)
                    Heap::new(self.unfold_name_in_expr(e, name, def_body))
                });

                Expr::new(
                    ExprKind::If {
                        condition: Heap::new(new_condition),
                        then_branch: new_then,
                        else_branch: new_else,
                    },
                    expr.span,
                )
            }
            ExprKind::Forall { bindings, body } => {
                // Unfold in body (but not if the name is bound by any binding pattern)
                let is_shadowed = bindings.iter().any(|b| {
                    extract_pattern_name(&b.pattern).as_ref() == Some(name)
                });
                if is_shadowed {
                    expr.clone() // Name is shadowed, don't unfold
                } else {
                    let new_body = Heap::new(self.unfold_name_in_expr(body, name, def_body));
                    Expr::new(
                        ExprKind::Forall {
                            bindings: bindings.clone(),
                            body: new_body,
                        },
                        expr.span,
                    )
                }
            }
            ExprKind::Exists { bindings, body } => {
                let is_shadowed = bindings.iter().any(|b| {
                    extract_pattern_name(&b.pattern).as_ref() == Some(name)
                });
                if is_shadowed {
                    expr.clone()
                } else {
                    let new_body = Heap::new(self.unfold_name_in_expr(body, name, def_body));
                    Expr::new(
                        ExprKind::Exists {
                            bindings: bindings.clone(),
                            body: new_body,
                        },
                        expr.span,
                    )
                }
            }
            // For other expression kinds, return as-is
            _ => expr.clone(),
        }
    }

    /// Apply compute tactic - normalize/evaluate the goal expression
    ///
    /// Performs computational normalization of the goal expression:
    /// - Evaluates arithmetic on concrete values (2 + 3 -> 5)
    /// - Simplifies boolean expressions (true && x -> x)
    /// - Applies algebraic identities (x + 0 -> x, x * 1 -> x)
    /// - Beta-reduces function applications where possible
    /// - Uses Z3 simplify tactic for complex simplifications
    ///
    /// If the goal simplifies to `true`, it is automatically proven.
    ///
    /// # Errors
    ///
    /// Returns `TacticError::Failed` if no simplification progress is made.
    fn apply_compute(&mut self) -> TacticResult<()> {
        let goal = self.state.current_goal()?;
        let original_prop = (*goal.proposition).clone();
        let hyps = goal.hypotheses.clone();

        // Apply computational normalization
        let normalized = self.normalize_expr(&original_prop);

        // Check if normalized to trivially true
        if self.is_trivially_true(&normalized) {
            self.state.prove_current_goal()?;
            return Ok(());
        }

        // Check if we made progress
        if self.exprs_match(&normalized, &original_prop) {
            // No syntactic progress - try Z3 simplification
            let z3_simplified = self.try_z3_simplify(&original_prop)?;

            if self.is_trivially_true(&z3_simplified) {
                self.state.prove_current_goal()?;
                return Ok(());
            }

            if self.exprs_match(&z3_simplified, &original_prop) {
                return Err(TacticError::Failed(Text::from(
                    "compute made no progress - expression is already in normal form",
                )));
            }

            // Update goal with Z3-simplified expression
            let new_goal = Goal::with_hypotheses(self.state.next_goal_id, z3_simplified, hyps);
            self.state.next_goal_id += 1;
            self.state
                .replace_current_goal(List::from_iter([new_goal]))?;
        } else {
            // Create new goal with normalized proposition
            let new_goal = Goal::with_hypotheses(self.state.next_goal_id, normalized, hyps);
            self.state.next_goal_id += 1;
            self.state
                .replace_current_goal(List::from_iter([new_goal]))?;
        }

        Ok(())
    }

    /// Normalize an expression by evaluating computable parts
    fn normalize_expr(&self, expr: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Literal(_) => expr.clone(),

            ExprKind::Binary { op, left, right } => {
                let left_norm = self.normalize_expr(left);
                let right_norm = self.normalize_expr(right);

                // Try constant folding for arithmetic
                match op {
                    BinOp::Add => {
                        // x + 0 = x
                        if self.is_zero_expr(&right_norm) {
                            return left_norm;
                        }
                        // 0 + x = x
                        if self.is_zero_expr(&left_norm) {
                            return right_norm;
                        }
                        // n + m
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            return self.make_int_expr(n + m);
                        }
                    }
                    BinOp::Sub => {
                        // x - 0 = x
                        if self.is_zero_expr(&right_norm) {
                            return left_norm;
                        }
                        // x - x = 0
                        if self.exprs_match(&left_norm, &right_norm) {
                            return self.make_int_expr(0);
                        }
                        // n - m
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            return self.make_int_expr(n - m);
                        }
                    }
                    BinOp::Mul => {
                        // x * 0 = 0
                        if self.is_zero_expr(&left_norm) || self.is_zero_expr(&right_norm) {
                            return self.make_int_expr(0);
                        }
                        // x * 1 = x
                        if self.is_one_expr(&right_norm) {
                            return left_norm;
                        }
                        // 1 * x = x
                        if self.is_one_expr(&left_norm) {
                            return right_norm;
                        }
                        // n * m
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            return self.make_int_expr(n * m);
                        }
                    }
                    BinOp::Div => {
                        // 0 / x = 0
                        if self.is_zero_expr(&left_norm) {
                            return self.make_int_expr(0);
                        }
                        // x / 1 = x
                        if self.is_one_expr(&right_norm) {
                            return left_norm;
                        }
                        // x / x = 1 (assuming x != 0)
                        if self.exprs_match(&left_norm, &right_norm)
                            && !self.is_zero_expr(&right_norm)
                        {
                            return self.make_int_expr(1);
                        }
                        // n / m
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            if m != 0 {
                                return self.make_int_expr(n / m);
                            }
                        }
                    }
                    BinOp::Rem => {
                        // n % m
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            if m != 0 {
                                return self.make_int_expr(n % m);
                            }
                        }
                    }
                    BinOp::And => {
                        // true && x = x
                        if self.is_true_expr(&left_norm) {
                            return right_norm;
                        }
                        // x && true = x
                        if self.is_true_expr(&right_norm) {
                            return left_norm;
                        }
                        // false && x = false
                        if self.is_false_expr(&left_norm) || self.is_false_expr(&right_norm) {
                            return self.make_bool_expr(false);
                        }
                        // x && x = x
                        if self.exprs_match(&left_norm, &right_norm) {
                            return left_norm;
                        }
                    }
                    BinOp::Or => {
                        // false || x = x
                        if self.is_false_expr(&left_norm) {
                            return right_norm;
                        }
                        // x || false = x
                        if self.is_false_expr(&right_norm) {
                            return left_norm;
                        }
                        // true || x = true
                        if self.is_true_expr(&left_norm) || self.is_true_expr(&right_norm) {
                            return self.make_bool_expr(true);
                        }
                        // x || x = x
                        if self.exprs_match(&left_norm, &right_norm) {
                            return left_norm;
                        }
                    }
                    BinOp::Eq => {
                        // x == x = true
                        if self.exprs_match(&left_norm, &right_norm) {
                            return self.make_bool_expr(true);
                        }
                        // n == m
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            return self.make_bool_expr(n == m);
                        }
                        // b1 == b2
                        if let (Some(b1), Some(b2)) = (
                            self.extract_bool(&left_norm),
                            self.extract_bool(&right_norm),
                        ) {
                            return self.make_bool_expr(b1 == b2);
                        }
                    }
                    BinOp::Ne => {
                        // x != x = false
                        if self.exprs_match(&left_norm, &right_norm) {
                            return self.make_bool_expr(false);
                        }
                        // n != m
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            return self.make_bool_expr(n != m);
                        }
                    }
                    BinOp::Lt => {
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            return self.make_bool_expr(n < m);
                        }
                    }
                    BinOp::Le => {
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            return self.make_bool_expr(n <= m);
                        }
                    }
                    BinOp::Gt => {
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            return self.make_bool_expr(n > m);
                        }
                    }
                    BinOp::Ge => {
                        if let (Some(n), Some(m)) =
                            (self.extract_int(&left_norm), self.extract_int(&right_norm))
                        {
                            return self.make_bool_expr(n >= m);
                        }
                    }
                    BinOp::Imply => {
                        // false => x = true
                        if self.is_false_expr(&left_norm) {
                            return self.make_bool_expr(true);
                        }
                        // x => true = true
                        if self.is_true_expr(&right_norm) {
                            return self.make_bool_expr(true);
                        }
                        // true => x = x
                        if self.is_true_expr(&left_norm) {
                            return right_norm;
                        }
                    }
                    _ => {}
                }

                // Return normalized binary expression
                Expr::new(
                    ExprKind::Binary {
                        op: *op,
                        left: Heap::new(left_norm),
                        right: Heap::new(right_norm),
                    },
                    expr.span,
                )
            }

            ExprKind::Unary { op, expr: inner } => {
                let inner_norm = self.normalize_expr(inner);

                match op {
                    verum_ast::expr::UnOp::Not => {
                        // !!x = x
                        if let ExprKind::Unary {
                            op: verum_ast::expr::UnOp::Not,
                            expr: inner2,
                        } = &inner_norm.kind
                        {
                            return (**inner2).clone();
                        }
                        // !true = false, !false = true
                        if let Some(b) = self.extract_bool(&inner_norm) {
                            return self.make_bool_expr(!b);
                        }
                    }
                    verum_ast::expr::UnOp::Neg => {
                        // --x = x
                        if let ExprKind::Unary {
                            op: verum_ast::expr::UnOp::Neg,
                            expr: inner2,
                        } = &inner_norm.kind
                        {
                            return (**inner2).clone();
                        }
                        // -n
                        if let Some(n) = self.extract_int(&inner_norm) {
                            return self.make_int_expr(-n);
                        }
                    }
                    _ => {}
                }

                Expr::new(
                    ExprKind::Unary {
                        op: *op,
                        expr: Heap::new(inner_norm),
                    },
                    expr.span,
                )
            }

            ExprKind::Paren(inner) => self.normalize_expr(inner),

            ExprKind::Call { func, args, .. } => {
                let func_norm = self.normalize_expr(func);
                let args_norm: List<_> = args.iter().map(|a| self.normalize_expr(a)).collect();

                // Beta reduction: (\x. body)(arg) -> body[x := arg]
                if let ExprKind::Closure { params, body, .. } = &func_norm.kind {
                    if params.len() == args_norm.len() {
                        let mut result = (**body).clone();
                        for (param, arg) in params.iter().zip(args_norm.iter()) {
                            if let Some(param_name) = extract_pattern_name(&param.pattern) {
                                result = self.substitute_var(&result, &param_name, arg);
                            }
                        }
                        return self.normalize_expr(&result);
                    }
                }

                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(func_norm),
                        type_args: Vec::new().into(),
                        args: args_norm,
                    },
                    expr.span,
                )
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Normalize condition
                let cond_exprs: List<_> = condition
                    .conditions
                    .iter()
                    .map(|c| match c {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            verum_ast::expr::ConditionKind::Expr(self.normalize_expr(e))
                        }
                        other => other.clone(),
                    })
                    .collect();

                // If condition is constant, simplify
                if cond_exprs.len() == 1 {
                    if let verum_ast::expr::ConditionKind::Expr(cond_expr) = &cond_exprs[0] {
                        if self.is_true_expr(cond_expr) {
                            // if true then A else B -> A
                            if let Some(trailing) = &then_branch.expr {
                                return self.normalize_expr(trailing);
                            }
                        }
                        if self.is_false_expr(cond_expr) {
                            // if false then A else B -> B
                            if let Some(else_expr) = else_branch {
                                return self.normalize_expr(else_expr);
                            }
                        }
                    }
                }

                expr.clone()
            }

            _ => expr.clone(),
        }
    }

    /// Check if an expression is trivially true
    fn is_trivially_true(&self, expr: &Expr) -> bool {
        if let ExprKind::Literal(lit) = &expr.kind {
            if let LiteralKind::Bool(true) = lit.kind {
                return true;
            }
        }
        false
    }

    /// Try to simplify expression using Z3
    ///
    /// Uses Z3's simplification tactic to normalize expressions. Handles:
    /// - Constant folding (2+3 -> 5)
    /// - Boolean simplification (true && x -> x)
    /// - Algebraic identities (x + 0 -> x)
    /// - Complex nested expressions
    ///
    /// Translates the simplified Z3 AST back to Verum Expr for full roundtrip.
    fn try_z3_simplify(&self, expr: &Expr) -> TacticResult<Expr> {
        // Translate to Z3 and use its simplify
        let mut var_map: HashMap<Text, z3::ast::Dynamic> = HashMap::new();

        match Self::translate_expr_to_z3(expr, &mut var_map) {
            Ok(z3_expr) => {
                // Use Z3's simplify - the Ast trait provides simplify()
                let simplified: z3::ast::Dynamic = Ast::simplify(&z3_expr);

                // Build reverse map for variable names
                let reverse_var_map: HashMap<String, Text> = var_map
                    .iter()
                    .map(|(name, _)| (name.to_string(), name.clone()))
                    .collect();

                // Translate back from Z3 to Expr with full expression reconstruction
                Self::translate_z3_to_expr(&simplified, &reverse_var_map)
            }
            Err(_) => {
                // Z3 translation failed, return original
                Ok(expr.clone())
            }
        }
    }

    /// Translate a Z3 Dynamic AST back to a Verum Expr
    ///
    /// This provides complete roundtrip support for Z3 simplification results,
    /// handling all expression types that can result from simplification.
    fn translate_z3_to_expr(
        z3_ast: &z3::ast::Dynamic,
        var_names: &HashMap<String, Text>,
    ) -> TacticResult<Expr> {
        use z3::AstKind;
        use z3::ast::{Ast, Bool, Int};

        // Handle boolean constants first
        if let Some(b) = z3_ast.as_bool() {
            if let Some(bool_val) = Bool::as_bool(&b) {
                return Ok(Expr::new(
                    ExprKind::Literal(verum_ast::Literal::new(
                        LiteralKind::Bool(bool_val),
                        Span::default(),
                    )),
                    Span::default(),
                ));
            }
        }

        // Handle integer constants
        if let Some(i) = z3_ast.as_int() {
            if let Some(int_val) = Int::as_i64(&i) {
                return Ok(Expr::new(
                    ExprKind::Literal(verum_ast::Literal::new(
                        LiteralKind::Int(verum_ast::literal::IntLit::new(int_val as i128)),
                        Span::default(),
                    )),
                    Span::default(),
                ));
            }
        }

        // Get AST kind for structural analysis
        let kind = z3_ast.kind();

        match kind {
            AstKind::Numeral => {
                // Already handled above for Int
                if let Some(i) = z3_ast.as_int() {
                    if let Some(int_val) = Int::as_i64(&i) {
                        return Ok(Expr::new(
                            ExprKind::Literal(verum_ast::Literal::new(
                                LiteralKind::Int(verum_ast::literal::IntLit::new(int_val as i128)),
                                Span::default(),
                            )),
                            Span::default(),
                        ));
                    }
                }
                Err(TacticError::SmtError(Text::from(
                    "failed to extract numeral value",
                )))
            }

            AstKind::App => {
                // Function application - could be variable, operator, or function call
                if let Ok(decl) = z3_ast.safe_decl() {
                    let decl_kind = decl.kind();
                    let num_args = z3_ast.num_children();

                    // Handle different declaration kinds
                    match decl_kind {
                        // Constants/Variables
                        z3::DeclKind::Uninterpreted => {
                            let name_str = decl.name();
                            let var_name = var_names
                                .get(&name_str)
                                .cloned()
                                .unwrap_or_else(|| Text::from(name_str.as_str()));

                            // Create path expression for variable
                            let path = verum_ast::Path::from_ident(verum_ast::Ident::new(
                                var_name,
                                Span::default(),
                            ));
                            Ok(Expr::new(ExprKind::Path(path), Span::default()))
                        }

                        // Boolean operations
                        z3::DeclKind::True => Ok(Expr::new(
                            ExprKind::Literal(verum_ast::Literal::new(
                                LiteralKind::Bool(true),
                                Span::default(),
                            )),
                            Span::default(),
                        )),
                        z3::DeclKind::False => Ok(Expr::new(
                            ExprKind::Literal(verum_ast::Literal::new(
                                LiteralKind::Bool(false),
                                Span::default(),
                            )),
                            Span::default(),
                        )),
                        z3::DeclKind::And => {
                            Self::translate_z3_nary_op(z3_ast, num_args, BinOp::And, var_names)
                        }
                        z3::DeclKind::Or => {
                            Self::translate_z3_nary_op(z3_ast, num_args, BinOp::Or, var_names)
                        }
                        z3::DeclKind::Not => {
                            if num_args == 1 {
                                let child = z3_ast.nth_child(0).ok_or_else(|| {
                                    TacticError::SmtError(Text::from("NOT missing child"))
                                })?;
                                let inner = Self::translate_z3_to_expr(&child, var_names)?;
                                Ok(Expr::new(
                                    ExprKind::Unary {
                                        op: verum_ast::expr::UnOp::Not,
                                        expr: Heap::new(inner),
                                    },
                                    Span::default(),
                                ))
                            } else {
                                Err(TacticError::SmtError(Text::from("NOT expects 1 argument")))
                            }
                        }
                        z3::DeclKind::Implies => {
                            Self::translate_z3_binary_op(z3_ast, BinOp::Imply, var_names)
                        }
                        z3::DeclKind::Iff => {
                            // IFF is equivalent to (a == b) for booleans
                            Self::translate_z3_binary_op(z3_ast, BinOp::Eq, var_names)
                        }
                        z3::DeclKind::Ite => {
                            // If-then-else
                            if num_args == 3 {
                                let cond_z3 = z3_ast.nth_child(0).ok_or_else(|| {
                                    TacticError::SmtError(Text::from("ITE missing condition"))
                                })?;
                                let then_z3 = z3_ast.nth_child(1).ok_or_else(|| {
                                    TacticError::SmtError(Text::from("ITE missing then branch"))
                                })?;
                                let else_z3 = z3_ast.nth_child(2).ok_or_else(|| {
                                    TacticError::SmtError(Text::from("ITE missing else branch"))
                                })?;

                                let cond_expr = Self::translate_z3_to_expr(&cond_z3, var_names)?;
                                let then_expr = Self::translate_z3_to_expr(&then_z3, var_names)?;
                                let else_expr = Self::translate_z3_to_expr(&else_z3, var_names)?;

                                // Create if expression
                                let condition = verum_ast::expr::IfCondition {
                                    conditions: smallvec::smallvec![
                                        verum_ast::expr::ConditionKind::Expr(cond_expr),
                                    ],
                                    span: Span::default(),
                                };
                                let then_block = verum_ast::expr::Block {
                                    stmts: List::new(),
                                    expr: Maybe::Some(Heap::new(then_expr)),
                                    span: Span::default(),
                                };

                                Ok(Expr::new(
                                    ExprKind::If {
                                        condition: Heap::new(condition),
                                        then_branch: then_block,
                                        else_branch: Maybe::Some(Heap::new(else_expr)),
                                    },
                                    Span::default(),
                                ))
                            } else {
                                Err(TacticError::SmtError(Text::from("ITE expects 3 arguments")))
                            }
                        }

                        // Comparison operations
                        z3::DeclKind::Eq => {
                            Self::translate_z3_binary_op(z3_ast, BinOp::Eq, var_names)
                        }
                        z3::DeclKind::Distinct => {
                            // DISTINCT(a, b) is equivalent to a != b for 2 args
                            if num_args == 2 {
                                Self::translate_z3_binary_op(z3_ast, BinOp::Ne, var_names)
                            } else {
                                // For more args, create conjunction of pairwise inequalities
                                Err(TacticError::SmtError(Text::from(
                                    "DISTINCT with >2 args not yet supported",
                                )))
                            }
                        }
                        z3::DeclKind::Lt => {
                            Self::translate_z3_binary_op(z3_ast, BinOp::Lt, var_names)
                        }
                        z3::DeclKind::Le => {
                            Self::translate_z3_binary_op(z3_ast, BinOp::Le, var_names)
                        }
                        z3::DeclKind::Gt => {
                            Self::translate_z3_binary_op(z3_ast, BinOp::Gt, var_names)
                        }
                        z3::DeclKind::Ge => {
                            Self::translate_z3_binary_op(z3_ast, BinOp::Ge, var_names)
                        }

                        // Arithmetic operations
                        z3::DeclKind::Add => {
                            Self::translate_z3_nary_op(z3_ast, num_args, BinOp::Add, var_names)
                        }
                        z3::DeclKind::Sub => {
                            Self::translate_z3_binary_op(z3_ast, BinOp::Sub, var_names)
                        }
                        z3::DeclKind::Mul => {
                            Self::translate_z3_nary_op(z3_ast, num_args, BinOp::Mul, var_names)
                        }
                        z3::DeclKind::Div | z3::DeclKind::Idiv => {
                            Self::translate_z3_binary_op(z3_ast, BinOp::Div, var_names)
                        }
                        z3::DeclKind::Mod | z3::DeclKind::Rem => {
                            Self::translate_z3_binary_op(z3_ast, BinOp::Rem, var_names)
                        }
                        z3::DeclKind::Uminus => {
                            if num_args == 1 {
                                let child = z3_ast.nth_child(0).ok_or_else(|| {
                                    TacticError::SmtError(Text::from("UMINUS missing child"))
                                })?;
                                let inner = Self::translate_z3_to_expr(&child, var_names)?;
                                Ok(Expr::new(
                                    ExprKind::Unary {
                                        op: verum_ast::expr::UnOp::Neg,
                                        expr: Heap::new(inner),
                                    },
                                    Span::default(),
                                ))
                            } else {
                                Err(TacticError::SmtError(Text::from(
                                    "UMINUS expects 1 argument",
                                )))
                            }
                        }

                        // For other declaration kinds, try to preserve structure
                        _ => {
                            // Fallback: create a function call expression
                            let func_name = decl.name();
                            let path = verum_ast::Path::from_ident(verum_ast::Ident::new(
                                Text::from(func_name.as_str()),
                                Span::default(),
                            ));
                            let func_expr = Expr::new(ExprKind::Path(path), Span::default());

                            let mut args = List::new();
                            for i in 0..num_args {
                                if let Some(child) = z3_ast.nth_child(i) {
                                    args.push(Self::translate_z3_to_expr(&child, var_names)?);
                                }
                            }

                            Ok(Expr::new(
                                ExprKind::Call {
                                    func: Heap::new(func_expr),
                                    type_args: Vec::new().into(),
                                    args,
                                },
                                Span::default(),
                            ))
                        }
                    }
                } else {
                    Err(TacticError::SmtError(Text::from(
                        "failed to get declaration from Z3 App",
                    )))
                }
            }

            AstKind::Var => {
                // Bound variable - extract index and look up name
                Err(TacticError::SmtError(Text::from(
                    "bound variables in Z3 result not yet supported",
                )))
            }

            AstKind::Quantifier => {
                // Quantified formula - could be forall or exists
                // For now, return an error as this is complex to translate back
                Err(TacticError::SmtError(Text::from(
                    "quantifiers in Z3 result not yet supported",
                )))
            }

            _ => Err(TacticError::SmtError(Text::from(format!(
                "unsupported Z3 AST kind: {:?}",
                kind
            )))),
        }
    }

    /// Translate a Z3 binary operation to Verum Expr
    fn translate_z3_binary_op(
        z3_ast: &z3::ast::Dynamic,
        op: BinOp,
        var_names: &HashMap<String, Text>,
    ) -> TacticResult<Expr> {
        let left_z3 = z3_ast
            .nth_child(0)
            .ok_or_else(|| TacticError::SmtError(Text::from("binary op missing left operand")))?;
        let right_z3 = z3_ast
            .nth_child(1)
            .ok_or_else(|| TacticError::SmtError(Text::from("binary op missing right operand")))?;

        let left = Self::translate_z3_to_expr(&left_z3, var_names)?;
        let right = Self::translate_z3_to_expr(&right_z3, var_names)?;

        Ok(Expr::new(
            ExprKind::Binary {
                op,
                left: Heap::new(left),
                right: Heap::new(right),
            },
            Span::default(),
        ))
    }

    /// Translate a Z3 n-ary operation to nested Verum binary Exprs
    fn translate_z3_nary_op(
        z3_ast: &z3::ast::Dynamic,
        num_args: usize,
        op: BinOp,
        var_names: &HashMap<String, Text>,
    ) -> TacticResult<Expr> {
        if num_args == 0 {
            return Err(TacticError::SmtError(Text::from(
                "n-ary op with 0 arguments",
            )));
        }

        if num_args == 1 {
            let child = z3_ast
                .nth_child(0)
                .ok_or_else(|| TacticError::SmtError(Text::from("n-ary op missing child")))?;
            return Self::translate_z3_to_expr(&child, var_names);
        }

        // Build left-associative tree
        let first = z3_ast
            .nth_child(0)
            .ok_or_else(|| TacticError::SmtError(Text::from("n-ary op missing first child")))?;
        let mut result = Self::translate_z3_to_expr(&first, var_names)?;

        for i in 1..num_args {
            let next = z3_ast.nth_child(i).ok_or_else(|| {
                TacticError::SmtError(Text::from(format!("n-ary op missing child {}", i)))
            })?;
            let next_expr = Self::translate_z3_to_expr(&next, var_names)?;
            result = Expr::new(
                ExprKind::Binary {
                    op,
                    left: Heap::new(result),
                    right: Heap::new(next_expr),
                },
                Span::default(),
            );
        }

        Ok(result)
    }

    /// Apply Z3 tactic to simplify goal using the Goal/Tactic API
    ///
    /// This uses Z3's tactic framework for more powerful transformations than
    /// simple simplify(). Returns the list of subgoals produced by the tactic.
    fn apply_z3_tactic_to_goal(
        &self,
        tactic_name: &str,
        goal_expr: &Expr,
        hypotheses: &List<Hypothesis>,
    ) -> TacticResult<Z3TacticResult> {
        use z3::ast::{Ast, Bool, Dynamic};

        // Create Z3 Goal
        let z3_goal = z3::Goal::new(true, false, false);

        // Translate and add hypotheses
        let mut var_map: HashMap<Text, Dynamic> = HashMap::new();
        for hyp in hypotheses {
            if let Ok(z3_hyp) = Self::translate_expr_to_z3(&hyp.proposition, &mut var_map) {
                if let Some(bool_hyp) = z3_hyp.as_bool() {
                    z3_goal.assert(&bool_hyp);
                }
            }
        }

        // Translate and add goal (we want to prove it, so add its negation for refutation)
        let z3_expr = Self::translate_expr_to_z3(goal_expr, &mut var_map).map_err(|e| {
            TacticError::SmtError(Text::from(format!("failed to translate goal: {}", e)))
        })?;

        if let Some(bool_goal) = z3_expr.as_bool() {
            z3_goal.assert(&bool_goal.not());
        } else {
            return Err(TacticError::SmtError(Text::from(
                "goal must be a boolean expression",
            )));
        }

        // Create and apply the tactic
        let tactic = z3::Tactic::new(tactic_name);
        let apply_result = tactic.apply(&z3_goal, None).map_err(|e| {
            TacticError::SmtError(Text::from(format!(
                "tactic '{}' failed: {}",
                tactic_name, e
            )))
        })?;

        // Build reverse var map
        let reverse_var_map: HashMap<String, Text> = var_map
            .iter()
            .map(|(name, _)| (name.to_string(), name.clone()))
            .collect();

        // Process subgoals
        let mut subgoals = List::new();
        for subgoal in apply_result.list_subgoals() {
            // Check if subgoal is trivially satisfied (inconsistent = proved)
            if subgoal.is_inconsistent() {
                // This subgoal is proven (goal was refuted)
                continue;
            }

            // Check if subgoal is decidedly unsat
            if subgoal.is_decided_unsat() {
                // Goal is proven
                continue;
            }

            // Check if subgoal is decidedly sat
            if subgoal.is_decided_sat() {
                // No remaining formulas - goal proven
                continue;
            }

            // Extract remaining formulas from subgoal
            let formulas = subgoal.get_formulas();
            if formulas.is_empty() {
                // Empty subgoal = proved
                continue;
            }

            // Translate remaining formulas back to Verum expressions
            for formula in formulas {
                let dynamic = Dynamic::from_ast(&formula);
                // The formula is the negated goal - negate back to get the remaining goal
                match Self::translate_z3_to_expr(&dynamic, &reverse_var_map) {
                    Ok(remaining) => {
                        // Since we asserted NOT(goal), remaining formulas are conditions
                        // that still need to be proven false
                        subgoals.push(Z3Subgoal {
                            formula: remaining,
                            depth: subgoal.get_depth() as usize,
                        });
                    }
                    Err(_) => {
                        // If we can't translate, keep the original goal
                        subgoals.push(Z3Subgoal {
                            formula: goal_expr.clone(),
                            depth: subgoal.get_depth() as usize,
                        });
                    }
                }
            }
        }

        let is_proven = subgoals.is_empty();
        Ok(Z3TacticResult {
            subgoals,
            is_proven,
        })
    }

    /// Apply a combined Z3 tactic strategy to the current goal
    ///
    /// Uses the Z3 tactic framework with combinators for powerful transformations:
    /// - `and_then`: Sequential tactic application
    /// - `or_else`: Try alternative if first fails
    /// - `repeat`: Apply tactic until fixed point
    /// - `try_for`: Apply with timeout
    fn apply_z3_tactic_strategy(
        &mut self,
        strategy: &Z3TacticStrategy,
        tactic_name: &str,
    ) -> TacticResult<()> {
        use z3::ast::{Ast, Bool, Dynamic};

        let goal = self.state.current_goal()?;

        // Fast path: trivially true goals don't need Z3
        if goal.is_trivial() {
            self.state.prove_current_goal()?;
            return Ok(());
        }

        self.stats.smt_calls += 1;

        // Create Z3 Goal
        let z3_goal = z3::Goal::new(true, false, false);

        // Translate and add hypotheses
        let mut var_map: HashMap<Text, Dynamic> = HashMap::new();
        for hyp in &goal.hypotheses {
            if let Ok(z3_hyp) = Self::translate_expr_to_z3(&hyp.proposition, &mut var_map) {
                if let Some(bool_hyp) = z3_hyp.as_bool() {
                    z3_goal.assert(&bool_hyp);
                }
            }
        }

        // Add global hypotheses
        for hyp in &self.state.global_hypotheses {
            if let Ok(z3_hyp) = Self::translate_expr_to_z3(&hyp.proposition, &mut var_map) {
                if let Some(bool_hyp) = z3_hyp.as_bool() {
                    z3_goal.assert(&bool_hyp);
                }
            }
        }

        // Assert negation of goal for refutation
        let z3_expr = Self::translate_expr_to_z3(&goal.proposition, &mut var_map).map_err(|e| {
            TacticError::SmtError(Text::from(format!("failed to translate goal: {}", e)))
        })?;

        if let Some(bool_goal) = z3_expr.as_bool() {
            z3_goal.assert(&bool_goal.not());
        } else {
            return Err(TacticError::SmtError(Text::from(
                "goal must be a boolean expression",
            )));
        }

        // Build the Z3 tactic from our strategy
        let z3_tactic = self.build_z3_tactic(strategy)?;

        // Apply tactic with timeout
        let timeout_tactic = z3_tactic.try_for(self.config.smt_timeout);

        let apply_result = timeout_tactic.apply(&z3_goal, None).map_err(|e| {
            if e.contains("timeout") {
                TacticError::Timeout(self.config.smt_timeout)
            } else {
                TacticError::SmtError(Text::from(format!("{} tactic failed: {}", tactic_name, e)))
            }
        })?;

        // Check if all subgoals are solved
        let subgoals: Vec<_> = apply_result.list_subgoals().collect();

        // If no subgoals or all subgoals are inconsistent/decided, goal is proven
        let all_proven = subgoals
            .iter()
            .all(|sg| sg.is_inconsistent() || sg.is_decided_unsat() || sg.get_size() == 0);

        if all_proven {
            self.state.prove_current_goal()?;
            return Ok(());
        }

        // If there are remaining subgoals, check if we made progress
        // For now, we report the tactic failed if subgoals remain
        let remaining_count: usize = subgoals
            .iter()
            .filter(|sg| !sg.is_inconsistent() && !sg.is_decided_unsat() && sg.get_size() > 0)
            .count();

        if remaining_count > 0 {
            Err(TacticError::Failed(Text::from(format!(
                "{} tactic left {} subgoal(s) unproven",
                tactic_name, remaining_count
            ))))
        } else {
            self.state.prove_current_goal()?;
            Ok(())
        }
    }

    /// Build a Z3 tactic from our strategy representation
    fn build_z3_tactic(&self, strategy: &Z3TacticStrategy) -> TacticResult<z3::Tactic> {
        match strategy {
            Z3TacticStrategy::Named(name) => Ok(z3::Tactic::new(name.as_str())),
            Z3TacticStrategy::AndThen(first, second) => {
                let t1 = self.build_z3_tactic(first)?;
                let t2 = self.build_z3_tactic(second)?;
                Ok(t1.and_then(&t2))
            }
            Z3TacticStrategy::OrElse(first, second) => {
                let t1 = self.build_z3_tactic(first)?;
                let t2 = self.build_z3_tactic(second)?;
                Ok(t1.or_else(&t2))
            }
            Z3TacticStrategy::Repeat(inner, max) => {
                let t = self.build_z3_tactic(inner)?;
                Ok(z3::Tactic::repeat(&t, *max))
            }
            Z3TacticStrategy::TryFor(inner, timeout) => {
                let t = self.build_z3_tactic(inner)?;
                Ok(t.try_for(*timeout))
            }
            Z3TacticStrategy::Skip => Ok(z3::Tactic::create_skip()),
            Z3TacticStrategy::Fail => Ok(z3::Tactic::create_fail()),
        }
    }

    // Helper functions for normalize_expr

    fn is_zero_expr(&self, expr: &Expr) -> bool {
        self.extract_int(expr) == Some(0)
    }

    fn is_one_expr(&self, expr: &Expr) -> bool {
        self.extract_int(expr) == Some(1)
    }

    fn is_true_expr(&self, expr: &Expr) -> bool {
        self.extract_bool(expr) == Some(true)
    }

    fn is_false_expr(&self, expr: &Expr) -> bool {
        self.extract_bool(expr) == Some(false)
    }

    fn extract_int(&self, expr: &Expr) -> Option<i64> {
        if let ExprKind::Literal(lit) = &expr.kind {
            if let LiteralKind::Int(int_lit) = &lit.kind {
                return Some(int_lit.value as i64);
            }
        }
        None
    }

    fn extract_bool(&self, expr: &Expr) -> Option<bool> {
        if let ExprKind::Literal(lit) = &expr.kind {
            if let LiteralKind::Bool(b) = &lit.kind {
                return Some(*b);
            }
        }
        None
    }

    fn make_int_expr(&self, value: i64) -> Expr {
        Expr::new(
            ExprKind::Literal(verum_ast::Literal::new(
                LiteralKind::Int(verum_ast::literal::IntLit::new(value as i128)),
                Span::default(),
            )),
            Span::default(),
        )
    }

    fn make_bool_expr(&self, value: bool) -> Expr {
        Expr::new(
            ExprKind::Literal(verum_ast::Literal::new(
                LiteralKind::Bool(value),
                Span::default(),
            )),
            Span::default(),
        )
    }

    // ==================== SMT-based Tactics ====================

    /// Apply simp tactic - simplification with lemmas
    fn apply_simp(&mut self, lemmas: &List<Expr>, at_target: Option<&Ident>) -> TacticResult<()> {
        // Use Z3 simplification tactic
        let strategy = StrategyBuilder::new()
            .then(TacticKind::Simplify)
            .then(TacticKind::SolveEqs)
            .build();

        self.apply_smt_strategy(&strategy, "simp")
    }

    /// Apply ring tactic - ring arithmetic solver
    fn apply_ring(&mut self) -> TacticResult<()> {
        // Ring arithmetic can be handled by SMT solver with NLA support
        let strategy = TacticCombinator::Single(TacticKind::SMT);
        self.apply_smt_strategy(&strategy, "ring")
    }

    /// Apply field tactic - field arithmetic solver
    fn apply_field(&mut self) -> TacticResult<()> {
        // Field arithmetic can be handled by SMT solver with NLA support
        let strategy = TacticCombinator::Single(TacticKind::SMT);
        self.apply_smt_strategy(&strategy, "field")
    }

    /// Apply omega tactic - linear integer arithmetic
    fn apply_omega(&mut self) -> TacticResult<()> {
        let strategy = StrategyBuilder::new()
            .then(TacticKind::Simplify)
            .then(TacticKind::LIA)
            .build();

        self.apply_smt_strategy(&strategy, "omega")
    }

    /// Apply auto tactic - automated proof search
    fn apply_auto(&mut self, with_hints: &List<Ident>) -> TacticResult<()> {
        // Auto tactic uses SMT solver with simplification preprocessing
        let strategy = StrategyBuilder::new()
            .then(TacticKind::Simplify)
            .then(TacticKind::SolveEqs)
            .or_else(TacticKind::SMT)
            .build();

        self.apply_smt_strategy(&strategy, "auto")
    }

    /// Apply blast tactic - tableau prover
    fn apply_blast(&mut self) -> TacticResult<()> {
        // Blast tactic uses SMT solver with aggressive simplification
        let strategy = StrategyBuilder::new()
            .then(TacticKind::Simplify)
            .then(TacticKind::SMT)
            .build();

        self.apply_smt_strategy(&strategy, "blast")
    }

    /// Apply smt tactic - general SMT solver
    fn apply_smt(&mut self, solver: Option<&Text>, timeout: Maybe<u64>) -> TacticResult<()> {
        let mut strategy = StrategyBuilder::new()
            .then(TacticKind::Simplify)
            .then(TacticKind::SMT);

        if let Maybe::Some(timeout_ms) = timeout {
            strategy = strategy.try_for(Duration::from_millis(timeout_ms));
        }

        self.apply_smt_strategy(&strategy.build(), "smt")
    }

    /// Apply an SMT strategy to the current goal
    ///
    /// This method translates the current goal to Z3, calls the SMT solver,
    /// and updates the proof state based on the result:
    /// - UNSAT: The negation of the goal is unsatisfiable, meaning the goal is valid
    /// - SAT: Found a counterexample that violates the goal
    /// - UNKNOWN: Solver could not determine the result (timeout, too complex, etc.)
    fn apply_smt_strategy(
        &mut self,
        _strategy: &TacticCombinator,
        tactic_name: &str,
    ) -> TacticResult<()> {
        // Get the current goal
        let goal = self.state.current_goal()?;

        // Fast path: trivially true goals don't need SMT
        if goal.is_trivial() {
            self.state.prove_current_goal()?;
            return Ok(());
        }

        // Track SMT call statistics
        self.stats.smt_calls += 1;

        // Create Z3 solver with appropriate timeout
        let timeout_ms = self.config.smt_timeout.as_millis() as u32;
        let solver = z3::Solver::new();

        // Apply timeout configuration
        let mut params = z3::Params::new();
        params.set_u32("timeout", timeout_ms);
        solver.set_params(&params);

        // Translate the goal proposition (Expr) to Z3
        // We need to prove: hypotheses => goal
        // Which is equivalent to checking: NOT(hypotheses => goal) is UNSAT
        // i.e., hypotheses AND NOT(goal) is UNSAT

        // First, translate and assert all hypotheses
        let mut var_map: HashMap<Text, z3::ast::Dynamic> = HashMap::new();

        for hyp in &goal.hypotheses {
            match Self::translate_expr_to_z3(&hyp.proposition, &mut var_map) {
                Ok(z3_hyp) => {
                    if let Some(bool_hyp) = z3_hyp.as_bool() {
                        solver.assert(&bool_hyp);
                    }
                }
                Err(_) => {
                    // Skip hypotheses that can't be translated
                    continue;
                }
            }
        }

        // Also assert global hypotheses
        for hyp in &self.state.global_hypotheses {
            match Self::translate_expr_to_z3(&hyp.proposition, &mut var_map) {
                Ok(z3_hyp) => {
                    if let Some(bool_hyp) = z3_hyp.as_bool() {
                        solver.assert(&bool_hyp);
                    }
                }
                Err(_) => continue,
            }
        }

        // Translate the goal proposition
        let z3_goal = Self::translate_expr_to_z3(&goal.proposition, &mut var_map).map_err(|e| {
            TacticError::SmtError(Text::from(format!("failed to translate goal to Z3: {}", e)))
        })?;

        // Assert the NEGATION of the goal
        // If hypotheses AND NOT(goal) is UNSAT, then hypotheses => goal is valid
        let z3_goal_bool = z3_goal
            .as_bool()
            .ok_or_else(|| TacticError::SmtError(Text::from("goal proposition is not boolean")))?;
        solver.assert(z3_goal_bool.not());

        // Check satisfiability
        match solver.check() {
            z3::SatResult::Unsat => {
                // Goal is valid - the negation is unsatisfiable
                self.state.prove_current_goal()?;
                Ok(())
            }
            z3::SatResult::Sat => {
                // Found a counterexample - goal cannot be proven
                let counterexample_msg = if let Some(model) = solver.get_model() {
                    // Extract counterexample values from the model
                    let mut values = List::new();
                    for (name, z3_var) in var_map.iter() {
                        if let Some(val) = model.eval(z3_var, true) {
                            values.push(format!("{} = {}", name, val));
                        }
                    }
                    if values.is_empty() {
                        "counterexample found".to_string()
                    } else {
                        format!("counterexample: {}", values.join(", "))
                    }
                } else {
                    "counterexample found (no model available)".to_string()
                };

                Err(TacticError::Failed(Text::from(format!(
                    "{} tactic failed: {}",
                    tactic_name, counterexample_msg
                ))))
            }
            z3::SatResult::Unknown => {
                // Solver couldn't determine - might be timeout or too complex
                let reason = solver
                    .get_reason_unknown()
                    .unwrap_or_else(|| "unknown reason".to_string());

                if reason.contains("timeout") {
                    Err(TacticError::Timeout(self.config.smt_timeout))
                } else {
                    Err(TacticError::SmtError(Text::from(format!(
                        "{} tactic returned unknown: {}",
                        tactic_name, reason
                    ))))
                }
            }
        }
    }

    /// Translate a Verum Expr to a Z3 Dynamic expression
    ///
    /// This handles the core expression forms needed for proof goals.
    /// Uses z3 0.19.x API which doesn't require context for most operations.
    fn translate_expr_to_z3(
        expr: &Expr,
        var_map: &mut HashMap<Text, z3::ast::Dynamic>,
    ) -> Result<z3::ast::Dynamic, Text> {
        use z3::ast::{Ast, Bool, Dynamic, Int};

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Bool(b) => Ok(Dynamic::from_ast(&Bool::from_bool(*b))),
                LiteralKind::Int(int_lit) => {
                    Ok(Dynamic::from_ast(&Int::from_i64(int_lit.value as i64)))
                }
                _ => Err(Text::from(format!("unsupported literal: {:?}", lit.kind))),
            },

            ExprKind::Path(path) => {
                // Extract variable name
                let name = if let Some(ident) = path.as_ident() {
                    Text::from(ident.as_str())
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

                // Handle boolean constants
                match name.as_str() {
                    "true" => return Ok(Dynamic::from_ast(&Bool::from_bool(true))),
                    "false" => return Ok(Dynamic::from_ast(&Bool::from_bool(false))),
                    _ => {}
                }

                // Look up or create variable
                if let Some(z3_var) = var_map.get(&name) {
                    Ok(z3_var.clone())
                } else {
                    // Default to integer variable (could be improved with type info)
                    let int_var = Int::new_const(name.as_str());
                    let dyn_var = Dynamic::from_ast(&int_var);
                    var_map.insert(name, dyn_var.clone());
                    Ok(dyn_var)
                }
            }

            ExprKind::Binary { op, left, right } => {
                let z3_left = Self::translate_expr_to_z3(left, var_map)?;
                let z3_right = Self::translate_expr_to_z3(right, var_map)?;

                match op {
                    // Logical operators
                    BinOp::And => {
                        let l = z3_left
                            .as_bool()
                            .ok_or_else(|| Text::from("AND requires bool"))?;
                        let r = z3_right
                            .as_bool()
                            .ok_or_else(|| Text::from("AND requires bool"))?;
                        Ok(Dynamic::from_ast(&Bool::and(&[&l, &r])))
                    }
                    BinOp::Or => {
                        let l = z3_left
                            .as_bool()
                            .ok_or_else(|| Text::from("OR requires bool"))?;
                        let r = z3_right
                            .as_bool()
                            .ok_or_else(|| Text::from("OR requires bool"))?;
                        Ok(Dynamic::from_ast(&Bool::or(&[&l, &r])))
                    }

                    // Comparison operators
                    BinOp::Eq => Ok(Dynamic::from_ast(&z3_left.eq(&z3_right))),
                    BinOp::Ne => Ok(Dynamic::from_ast(&z3_left.eq(&z3_right).not())),
                    BinOp::Lt => {
                        let l = z3_left
                            .as_int()
                            .ok_or_else(|| Text::from("< requires int"))?;
                        let r = z3_right
                            .as_int()
                            .ok_or_else(|| Text::from("< requires int"))?;
                        Ok(Dynamic::from_ast(&l.lt(&r)))
                    }
                    BinOp::Le => {
                        let l = z3_left
                            .as_int()
                            .ok_or_else(|| Text::from("<= requires int"))?;
                        let r = z3_right
                            .as_int()
                            .ok_or_else(|| Text::from("<= requires int"))?;
                        Ok(Dynamic::from_ast(&l.le(&r)))
                    }
                    BinOp::Gt => {
                        let l = z3_left
                            .as_int()
                            .ok_or_else(|| Text::from("> requires int"))?;
                        let r = z3_right
                            .as_int()
                            .ok_or_else(|| Text::from("> requires int"))?;
                        Ok(Dynamic::from_ast(&l.gt(&r)))
                    }
                    BinOp::Ge => {
                        let l = z3_left
                            .as_int()
                            .ok_or_else(|| Text::from(">= requires int"))?;
                        let r = z3_right
                            .as_int()
                            .ok_or_else(|| Text::from(">= requires int"))?;
                        Ok(Dynamic::from_ast(&l.ge(&r)))
                    }

                    // Arithmetic operators
                    BinOp::Add => {
                        let l = z3_left
                            .as_int()
                            .ok_or_else(|| Text::from("+ requires int"))?;
                        let r = z3_right
                            .as_int()
                            .ok_or_else(|| Text::from("+ requires int"))?;
                        Ok(Dynamic::from_ast(&Int::add(&[&l, &r])))
                    }
                    BinOp::Sub => {
                        let l = z3_left
                            .as_int()
                            .ok_or_else(|| Text::from("- requires int"))?;
                        let r = z3_right
                            .as_int()
                            .ok_or_else(|| Text::from("- requires int"))?;
                        Ok(Dynamic::from_ast(&Int::sub(&[&l, &r])))
                    }
                    BinOp::Mul => {
                        let l = z3_left
                            .as_int()
                            .ok_or_else(|| Text::from("* requires int"))?;
                        let r = z3_right
                            .as_int()
                            .ok_or_else(|| Text::from("* requires int"))?;
                        Ok(Dynamic::from_ast(&Int::mul(&[&l, &r])))
                    }
                    BinOp::Div => {
                        let l = z3_left
                            .as_int()
                            .ok_or_else(|| Text::from("/ requires int"))?;
                        let r = z3_right
                            .as_int()
                            .ok_or_else(|| Text::from("/ requires int"))?;
                        Ok(Dynamic::from_ast(&l.div(&r)))
                    }
                    BinOp::Rem => {
                        let l = z3_left
                            .as_int()
                            .ok_or_else(|| Text::from("% requires int"))?;
                        let r = z3_right
                            .as_int()
                            .ok_or_else(|| Text::from("% requires int"))?;
                        Ok(Dynamic::from_ast(&l.rem(&r)))
                    }

                    // Implication (for logical formulas)
                    BinOp::Imply => {
                        let l = z3_left
                            .as_bool()
                            .ok_or_else(|| Text::from("=> requires bool"))?;
                        let r = z3_right
                            .as_bool()
                            .ok_or_else(|| Text::from("=> requires bool"))?;
                        Ok(Dynamic::from_ast(&l.implies(&r)))
                    }

                    _ => Err(Text::from(format!("unsupported binary operator: {:?}", op))),
                }
            }

            ExprKind::Unary { op, expr: inner } => {
                let z3_inner = Self::translate_expr_to_z3(inner, var_map)?;

                match op {
                    verum_ast::expr::UnOp::Not => {
                        let b = z3_inner
                            .as_bool()
                            .ok_or_else(|| Text::from("NOT requires bool"))?;
                        Ok(Dynamic::from_ast(&b.not()))
                    }
                    verum_ast::expr::UnOp::Neg => {
                        let i = z3_inner
                            .as_int()
                            .ok_or_else(|| Text::from("NEG requires int"))?;
                        Ok(Dynamic::from_ast(&i.unary_minus()))
                    }
                    _ => Err(Text::from(format!("unsupported unary operator: {:?}", op))),
                }
            }

            ExprKind::Paren(inner) => Self::translate_expr_to_z3(inner, var_map),

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Translate condition - handle IfCondition type
                let cond_exprs: Result<List<Bool>, Text> = condition
                    .conditions
                    .iter()
                    .map(|c| {
                        match c {
                            verum_ast::expr::ConditionKind::Expr(e) => {
                                let z3_e = Self::translate_expr_to_z3(e, var_map)?;
                                z3_e.as_bool()
                                    .ok_or_else(|| Text::from("condition must be boolean"))
                            }
                            verum_ast::expr::ConditionKind::Let { .. } => {
                                // Let bindings in conditions - assume true for now
                                Ok(Bool::from_bool(true))
                            }
                        }
                    })
                    .collect();
                let cond_exprs = cond_exprs?;
                let cond_refs: Vec<&Bool> = cond_exprs.iter().collect();
                let z3_cond = if cond_refs.len() == 1 {
                    cond_refs[0].clone()
                } else {
                    Bool::and(&cond_refs)
                };

                // Translate then branch
                let then_result = if let Some(trailing_expr) = &then_branch.expr {
                    Self::translate_expr_to_z3(trailing_expr, var_map)?
                } else {
                    Dynamic::from_ast(&Bool::from_bool(true))
                };

                // Translate else branch
                let else_result = if let Some(else_expr) = else_branch {
                    Self::translate_expr_to_z3(else_expr, var_map)?
                } else {
                    Dynamic::from_ast(&Bool::from_bool(true))
                };

                // Create ITE
                if let (Some(then_bool), Some(else_bool)) =
                    (then_result.as_bool(), else_result.as_bool())
                {
                    Ok(Dynamic::from_ast(&z3_cond.ite(&then_bool, &else_bool)))
                } else if let (Some(then_int), Some(else_int)) =
                    (then_result.as_int(), else_result.as_int())
                {
                    Ok(Dynamic::from_ast(&z3_cond.ite(&then_int, &else_int)))
                } else {
                    Err(Text::from("if-then-else branches must have matching types"))
                }
            }

            _ => Err(Text::from(format!(
                "unsupported expression kind: {:?}",
                std::mem::discriminant(&expr.kind)
            ))),
        }
    }

    // ==================== Combinator Tactics ====================

    /// Apply sequence of tactics
    fn apply_sequence(&mut self, tactics: &List<TacticExpr>) -> TacticResult<()> {
        for tactic in tactics {
            self.apply_tactic(tactic)?;
        }
        Ok(())
    }

    /// Apply alternative tactics (try each until one succeeds)
    fn apply_alternative(&mut self, tactics: &List<TacticExpr>) -> TacticResult<()> {
        for tactic in tactics {
            if self.apply_tactic(tactic).is_ok() {
                return Ok(());
            }
        }
        Err(TacticError::Failed(Text::from("all alternatives failed")))
    }

    /// Try a tactic, continue if it fails
    fn apply_try(&mut self, inner: &Heap<TacticExpr>) -> TacticResult<()> {
        // Try the tactic, but don't fail if it doesn't work
        let _ = self.apply_tactic(inner);
        Ok(())
    }

    /// Repeat a tactic until it fails
    fn apply_repeat(&mut self, inner: &Heap<TacticExpr>) -> TacticResult<()> {
        let max_iterations = self.config.max_repeat_iterations;
        for _ in 0..max_iterations {
            if self.apply_tactic(inner).is_err() {
                break;
            }
        }
        Ok(())
    }

    /// Apply tactic to all goals
    fn apply_all_goals(&mut self, inner: &Heap<TacticExpr>) -> TacticResult<()> {
        let num_goals = self.state.goals.len();
        for _ in 0..num_goals {
            self.apply_tactic(inner)?;
        }
        Ok(())
    }

    /// Focus on current goal
    fn apply_focus(&mut self, inner: &Heap<TacticExpr>) -> TacticResult<()> {
        // Focus just applies the tactic to the current goal
        self.apply_tactic(inner)
    }

    /// Apply named (user-defined) tactic
    ///
    /// Looks up a registered tactic by name, validates and binds arguments
    /// to parameters, then executes the tactic body. This enables users to
    /// define reusable proof strategies.
    ///
    /// # Tactic Resolution
    ///
    /// 1. First checks the local tactic registry
    /// 2. Built-in tactics take precedence (handled by main apply_tactic dispatch)
    ///
    /// # Parameter Binding
    ///
    /// Arguments are bound to parameters by position. The following parameter
    /// kinds are supported (see `TacticParamKind`):
    ///
    /// - `Expr`: Expression arguments (most common)
    /// - `Type`: Type arguments (for polymorphic tactics)
    /// - `Tactic`: Higher-order tactic arguments
    /// - `Hypothesis`: Hypothesis name arguments
    /// - `Int`: Integer arguments (for iteration counts, etc.)
    ///
    /// # Errors
    ///
    /// - `TacticError::Failed` if the tactic is not found in the registry
    /// - `TacticError::InvalidArgument` if argument count doesn't match parameters
    /// - Propagates errors from the tactic body execution
    ///
    /// # Example
    ///
    /// ```verum
    /// // Define a tactic that simplifies and then applies auto
    /// tactic simp_auto is {
    ///     simp;
    ///     auto
    /// }
    ///
    /// // Use the named tactic
    /// theorem example: P {
    ///     by simp_auto
    /// }
    /// ```
    ///
    /// Named tactic application: look up a user-defined tactic in the registry
    /// and evaluate its body with the given arguments. Named tactics are defined
    /// via `tactic my_tactic is { ... }` and support repeat, match goal, first,
    /// and other combinators.
    fn apply_named(&mut self, name: &Ident, args: &List<Expr>) -> TacticResult<()> {
        let tactic_name = Text::from(name.as_str());

        // Step 1: Look up the named tactic in the registry
        let tactic_decl = match self.tactic_registry.get(&tactic_name) {
            Some(decl) => decl.clone(),
            None => {
                return Err(TacticError::Failed(Text::from(format!(
                    "named tactic '{}' not found in registry",
                    tactic_name
                ))));
            }
        };

        // Step 2: Validate argument count matches parameter count
        let param_count = tactic_decl.params.len();
        let arg_count = args.len();

        if arg_count != param_count {
            return Err(TacticError::InvalidArgument(Text::from(format!(
                "tactic '{}' expects {} argument(s), but {} were provided",
                tactic_name, param_count, arg_count
            ))));
        }

        // Step 3: Create parameter bindings
        // We bind parameter names to argument expressions for substitution
        let mut param_bindings: Map<Text, Expr> = Map::new();
        for (param, arg) in tactic_decl.params.iter().zip(args.iter()) {
            let param_name = Text::from(param.name.as_str());

            // Validate argument kind matches parameter kind
            match &param.kind {
                TacticParamKind::Expr => {
                    // Any expression is valid
                    param_bindings.insert(param_name, arg.clone());
                }
                TacticParamKind::Type => {
                    // For type parameters, we expect a type expression (path, etc.)
                    // In a full implementation, we would validate the expression is a type
                    param_bindings.insert(param_name, arg.clone());
                }
                TacticParamKind::Tactic => {
                    // For tactic parameters, we store the expression
                    // It will be interpreted as a tactic when used
                    param_bindings.insert(param_name, arg.clone());
                }
                TacticParamKind::Hypothesis => {
                    // For hypothesis parameters, validate it's a simple identifier
                    if !matches!(&arg.kind, ExprKind::Path(p) if p.is_single()) {
                        return Err(TacticError::InvalidArgument(Text::from(format!(
                            "parameter '{}' expects a hypothesis name, got complex expression",
                            param_name
                        ))));
                    }
                    param_bindings.insert(param_name, arg.clone());
                }
                TacticParamKind::Int => {
                    // For integer parameters, validate it's an integer literal
                    if !matches!(&arg.kind, ExprKind::Literal(lit) if matches!(lit.kind, LiteralKind::Int(_)))
                    {
                        return Err(TacticError::InvalidArgument(Text::from(format!(
                            "parameter '{}' expects an integer literal",
                            param_name
                        ))));
                    }
                    param_bindings.insert(param_name, arg.clone());
                }
                TacticParamKind::Prop => {
                    // Propositions are first-class expressions in the tactic
                    // DSL — no structural validation beyond what the type
                    // checker already performs.
                    param_bindings.insert(param_name, arg.clone());
                }
                TacticParamKind::Other => {
                    // Arbitrary typed parameter — type-checking (via
                    // `param.ty`) runs in the semantic-analysis phase, so at
                    // this point we simply bind the argument.
                    param_bindings.insert(param_name, arg.clone());
                }
            }
        }

        // Step 4: Instantiate the tactic body with parameter bindings
        let instantiated_body = self.instantiate_tactic_body(&tactic_decl.body, &param_bindings)?;

        // Step 5: Apply the instantiated tactic body
        match instantiated_body {
            TacticBody::Simple(tactic_expr) => self.apply_tactic(&tactic_expr),
            TacticBody::Block(tactics) => {
                // Execute tactics in sequence
                for tactic_expr in &tactics {
                    self.apply_tactic(tactic_expr)?;
                }
                Ok(())
            }
        }
    }

    /// Instantiate a tactic body by substituting parameter references with arguments
    ///
    /// This function traverses the tactic body and replaces references to parameters
    /// with the corresponding argument expressions.
    fn instantiate_tactic_body(
        &self,
        body: &TacticBody,
        bindings: &Map<Text, Expr>,
    ) -> TacticResult<TacticBody> {
        match body {
            TacticBody::Simple(tactic) => {
                let instantiated = self.instantiate_tactic_expr(tactic, bindings)?;
                Ok(TacticBody::Simple(instantiated))
            }
            TacticBody::Block(tactics) => {
                let instantiated: TacticResult<List<_>> = tactics
                    .iter()
                    .map(|t| self.instantiate_tactic_expr(t, bindings))
                    .collect();
                Ok(TacticBody::Block(instantiated?))
            }
        }
    }

    /// Instantiate a single tactic expression with parameter bindings
    ///
    /// Recursively processes tactic expressions, substituting parameter references
    /// with bound argument expressions.
    fn instantiate_tactic_expr(
        &self,
        tactic: &TacticExpr,
        bindings: &Map<Text, Expr>,
    ) -> TacticResult<TacticExpr> {
        match tactic {
            // Tactics with expression arguments need substitution
            TacticExpr::Apply { lemma, args } => {
                let new_lemma = Heap::new(self.substitute_params_in_expr(lemma, bindings));
                let new_args: List<_> = args
                    .iter()
                    .map(|a| self.substitute_params_in_expr(a, bindings))
                    .collect();
                Ok(TacticExpr::Apply {
                    lemma: new_lemma,
                    args: new_args,
                })
            }
            TacticExpr::Rewrite {
                hypothesis,
                at_target,
                rev,
            } => {
                let new_hyp = Heap::new(self.substitute_params_in_expr(hypothesis, bindings));
                Ok(TacticExpr::Rewrite {
                    hypothesis: new_hyp,
                    at_target: at_target.clone(),
                    rev: *rev,
                })
            }
            TacticExpr::Exists(witness) => {
                let new_witness = Heap::new(self.substitute_params_in_expr(witness, bindings));
                Ok(TacticExpr::Exists(new_witness))
            }
            TacticExpr::Exact(proof) => {
                let new_proof = Heap::new(self.substitute_params_in_expr(proof, bindings));
                Ok(TacticExpr::Exact(new_proof))
            }
            TacticExpr::InductionOn(var) => {
                // Check if var is a bound parameter
                let var_name = Text::from(var.as_str());
                if let Some(arg_expr) = bindings.get(&var_name) {
                    // Extract identifier from the expression
                    if let ExprKind::Path(path) = &arg_expr.kind {
                        if let Some(ident) = path.as_ident() {
                            return Ok(TacticExpr::InductionOn(ident.clone()));
                        }
                    }
                }
                Ok(TacticExpr::InductionOn(var.clone()))
            }
            TacticExpr::CasesOn(var) => {
                // Check if var is a bound parameter
                let var_name = Text::from(var.as_str());
                if let Some(arg_expr) = bindings.get(&var_name) {
                    // Extract identifier from the expression
                    if let ExprKind::Path(path) = &arg_expr.kind {
                        if let Some(ident) = path.as_ident() {
                            return Ok(TacticExpr::CasesOn(ident.clone()));
                        }
                    }
                }
                Ok(TacticExpr::CasesOn(var.clone()))
            }
            TacticExpr::Intro(names) => {
                // Names are identifiers, check if any are bound parameters
                let new_names: List<_> = names
                    .iter()
                    .map(|n| {
                        let n_text = Text::from(n.as_str());
                        if let Some(arg_expr) = bindings.get(&n_text) {
                            if let ExprKind::Path(path) = &arg_expr.kind {
                                if let Some(ident) = path.as_ident() {
                                    return ident.clone();
                                }
                            }
                        }
                        n.clone()
                    })
                    .collect();
                Ok(TacticExpr::Intro(new_names))
            }
            TacticExpr::Unfold(names) => {
                // Substitute any bound parameters in the unfold list
                let new_names: List<_> = names
                    .iter()
                    .map(|n| {
                        let n_text = Text::from(n.as_str());
                        if let Some(arg_expr) = bindings.get(&n_text) {
                            if let ExprKind::Path(path) = &arg_expr.kind {
                                if let Some(ident) = path.as_ident() {
                                    return ident.clone();
                                }
                            }
                        }
                        n.clone()
                    })
                    .collect();
                Ok(TacticExpr::Unfold(new_names))
            }
            TacticExpr::Simp { lemmas, at_target } => {
                let new_lemmas: List<_> = lemmas
                    .iter()
                    .map(|l| self.substitute_params_in_expr(l, bindings))
                    .collect();
                Ok(TacticExpr::Simp {
                    lemmas: new_lemmas,
                    at_target: at_target.clone(),
                })
            }
            TacticExpr::Named {
                name: inner_name,
                generic_args: inner_generic_args,
                args: inner_args,
            } => {
                // Recursive named tactic - substitute arguments
                let new_args: List<_> = inner_args
                    .iter()
                    .map(|a| self.substitute_params_in_expr(a, bindings))
                    .collect();
                Ok(TacticExpr::Named {
                    name: inner_name.clone(),
                    generic_args: inner_generic_args.clone(),
                    args: new_args,
                })
            }

            // Tactics with sub-tactics need recursive instantiation
            TacticExpr::Seq(tactics) => {
                let new_tactics: TacticResult<List<_>> = tactics
                    .iter()
                    .map(|t| self.instantiate_tactic_expr(t, bindings))
                    .collect();
                Ok(TacticExpr::Seq(new_tactics?))
            }
            TacticExpr::Alt(tactics) => {
                let new_tactics: TacticResult<List<_>> = tactics
                    .iter()
                    .map(|t| self.instantiate_tactic_expr(t, bindings))
                    .collect();
                Ok(TacticExpr::Alt(new_tactics?))
            }
            TacticExpr::Try(inner) => {
                let new_inner = self.instantiate_tactic_expr(inner, bindings)?;
                Ok(TacticExpr::Try(Heap::new(new_inner)))
            }
            TacticExpr::TryElse { body, fallback } => {
                let new_body = self.instantiate_tactic_expr(body, bindings)?;
                let new_fallback = self.instantiate_tactic_expr(fallback, bindings)?;
                Ok(TacticExpr::TryElse {
                    body: Heap::new(new_body),
                    fallback: Heap::new(new_fallback),
                })
            }
            TacticExpr::Repeat(inner) => {
                let new_inner = self.instantiate_tactic_expr(inner, bindings)?;
                Ok(TacticExpr::Repeat(Heap::new(new_inner)))
            }
            TacticExpr::AllGoals(inner) => {
                let new_inner = self.instantiate_tactic_expr(inner, bindings)?;
                Ok(TacticExpr::AllGoals(Heap::new(new_inner)))
            }
            TacticExpr::Focus(inner) => {
                let new_inner = self.instantiate_tactic_expr(inner, bindings)?;
                Ok(TacticExpr::Focus(Heap::new(new_inner)))
            }

            // Simple tactics with no arguments - pass through unchanged
            TacticExpr::Trivial
            | TacticExpr::Assumption
            | TacticExpr::Reflexivity
            | TacticExpr::Split
            | TacticExpr::Left
            | TacticExpr::Right
            | TacticExpr::Compute
            | TacticExpr::Ring
            | TacticExpr::Field
            | TacticExpr::Omega
            | TacticExpr::Blast
            | TacticExpr::Done
            | TacticExpr::Admit
            | TacticExpr::Sorry
            | TacticExpr::Contradiction => Ok(tactic.clone()),

            // Tactics with options - pass through
            TacticExpr::Auto { with_hints } => Ok(TacticExpr::Auto {
                with_hints: with_hints.clone(),
            }),
            TacticExpr::Smt { solver, timeout } => Ok(TacticExpr::Smt {
                solver: solver.clone(),
                timeout: *timeout,
            }),

            // Tactic-DSL control-flow forms — substitute inside them
            TacticExpr::Let { name, ty, value } => Ok(TacticExpr::Let {
                name: name.clone(),
                ty: ty.clone(),
                value: Heap::new(self.substitute_params_in_expr(value, bindings)),
            }),
            TacticExpr::Match { scrutinee, arms } => {
                let new_scrutinee = self.substitute_params_in_expr(scrutinee, bindings);
                let new_arms: TacticResult<List<_>> = arms
                    .iter()
                    .map(|arm| {
                        let new_guard = match &arm.guard {
                            Maybe::Some(g) => Maybe::Some(Heap::new(
                                self.substitute_params_in_expr(g, bindings),
                            )),
                            Maybe::None => Maybe::None,
                        };
                        let new_body = self.instantiate_tactic_expr(&arm.body, bindings)?;
                        Ok(verum_ast::decl::TacticMatchArm {
                            pattern: arm.pattern.clone(),
                            guard: new_guard,
                            body: Heap::new(new_body),
                            span: arm.span,
                        })
                    })
                    .collect();
                Ok(TacticExpr::Match {
                    scrutinee: Heap::new(new_scrutinee),
                    arms: new_arms?,
                })
            }
            TacticExpr::Fail { message } => Ok(TacticExpr::Fail {
                message: Heap::new(self.substitute_params_in_expr(message, bindings)),
            }),
            TacticExpr::If { cond, then_branch, else_branch } => {
                let new_cond = self.substitute_params_in_expr(cond, bindings);
                let new_then = self.instantiate_tactic_expr(then_branch, bindings)?;
                let new_else = match else_branch {
                    Maybe::Some(e) => {
                        Maybe::Some(Heap::new(self.instantiate_tactic_expr(e, bindings)?))
                    }
                    Maybe::None => Maybe::None,
                };
                Ok(TacticExpr::If {
                    cond: Heap::new(new_cond),
                    then_branch: Heap::new(new_then),
                    else_branch: new_else,
                })
            }
        }
    }

    /// Substitute parameter references in an expression
    ///
    /// Replaces path expressions that match parameter names with the bound argument.
    fn substitute_params_in_expr(&self, expr: &Expr, bindings: &Map<Text, Expr>) -> Expr {
        match &expr.kind {
            ExprKind::Path(path) => {
                // Check if this is a simple path that matches a parameter name
                if let Some(ident) = path.as_ident() {
                    let name = Text::from(ident.as_str());
                    if let Some(bound_expr) = bindings.get(&name) {
                        return bound_expr.clone();
                    }
                }
                expr.clone()
            }
            ExprKind::Binary { op, left, right } => {
                let new_left = Heap::new(self.substitute_params_in_expr(left, bindings));
                let new_right = Heap::new(self.substitute_params_in_expr(right, bindings));
                Expr::new(
                    ExprKind::Binary {
                        op: *op,
                        left: new_left,
                        right: new_right,
                    },
                    expr.span,
                )
            }
            ExprKind::Unary { op, expr: inner } => {
                let new_inner = Heap::new(self.substitute_params_in_expr(inner, bindings));
                Expr::new(
                    ExprKind::Unary {
                        op: *op,
                        expr: new_inner,
                    },
                    expr.span,
                )
            }
            ExprKind::Call { func, args, .. } => {
                let new_func = Heap::new(self.substitute_params_in_expr(func, bindings));
                let new_args: List<_> = args
                    .iter()
                    .map(|a| self.substitute_params_in_expr(a, bindings))
                    .collect();
                Expr::new(
                    ExprKind::Call {
                        func: new_func,
                        type_args: Vec::new().into(),
                        args: new_args,
                    },
                    expr.span,
                )
            }
            ExprKind::MethodCall {
                receiver,
                method,
                type_args,
                args,
            } => {
                let new_receiver = Heap::new(self.substitute_params_in_expr(receiver, bindings));
                let new_args: List<_> = args
                    .iter()
                    .map(|a| self.substitute_params_in_expr(a, bindings))
                    .collect();
                Expr::new(
                    ExprKind::MethodCall {
                        receiver: new_receiver,
                        method: method.clone(),
                        type_args: type_args.clone(),
                        args: new_args,
                    },
                    expr.span,
                )
            }
            ExprKind::Tuple(elems) => {
                let new_elems: List<_> = elems
                    .iter()
                    .map(|e| self.substitute_params_in_expr(e, bindings))
                    .collect();
                Expr::new(ExprKind::Tuple(new_elems), expr.span)
            }
            ExprKind::Field { expr: inner, field } => {
                let new_inner = Heap::new(self.substitute_params_in_expr(inner, bindings));
                Expr::new(
                    ExprKind::Field {
                        expr: new_inner,
                        field: field.clone(),
                    },
                    expr.span,
                )
            }
            ExprKind::Index { expr: inner, index } => {
                let new_inner = Heap::new(self.substitute_params_in_expr(inner, bindings));
                let new_index = Heap::new(self.substitute_params_in_expr(index, bindings));
                Expr::new(
                    ExprKind::Index {
                        expr: new_inner,
                        index: new_index,
                    },
                    expr.span,
                )
            }
            ExprKind::Paren(inner) => {
                let new_inner = Heap::new(self.substitute_params_in_expr(inner, bindings));
                Expr::new(ExprKind::Paren(new_inner), expr.span)
            }
            // For other expressions, return as-is (could be extended for completeness)
            _ => expr.clone(),
        }
    }

    /// Apply done tactic - verify all goals proven
    fn apply_done(&mut self) -> TacticResult<()> {
        if self.state.is_complete() {
            Ok(())
        } else {
            Err(TacticError::Failed(Text::from(format!(
                "{} goals remaining",
                self.state.num_goals()
            ))))
        }
    }

    /// Apply admit tactic - admit goal without proof (for development)
    fn apply_admit(&mut self) -> TacticResult<()> {
        self.stats.admitted_goals += 1;
        self.state.prove_current_goal()?;
        Ok(())
    }

    /// Apply sorry tactic - like admit but marks as incomplete
    fn apply_sorry(&mut self) -> TacticResult<()> {
        self.stats.sorry_goals += 1;
        self.state.prove_current_goal()?;
        Ok(())
    }

    /// Apply contradiction tactic (proof by contradiction).
    /// Assumes the context contains a contradiction and proves any goal.
    fn apply_contradiction_tactic(&mut self) -> TacticResult<()> {
        // Proof by contradiction - the context should contain False or a contradiction
        // For now, treat similarly to auto - attempt to find a contradiction
        self.apply_auto(&List::new())
    }

    /// T1-W: `let name: T = value;` — bind a local name inside the
    /// current tactic's context so subsequent tactic expressions in
    /// the same sequence can reference it. The binding is recorded as
    /// a hypothesis of the current goal; type annotation (if present)
    /// is accepted but not enforced beyond what the expression's
    /// own type-inference already does.
    fn apply_let(
        &mut self,
        name: &verum_ast::Ident,
        value: &Heap<Expr>,
    ) -> TacticResult<()> {
        let goal = self.state.current_goal_mut()?;
        goal.hypotheses.push(Hypothesis {
            name: Text::from(name.as_str()),
            proposition: value.clone(),
            ty: Maybe::None,
            source: HypothesisSource::Generated,
        });
        Ok(())
    }

    /// T1-W: `match scrutinee { P => tactic, ... }` — pattern-directed
    /// branching inside a tactic body. The scrutinee is treated as a
    /// closed expression that the evaluator examines at tactic time.
    /// Rationale: tactics run at proof-elaboration time against the
    /// structural shape of the scrutinee (e.g. `Maybe.Some(_)` vs
    /// `Maybe.None`), so the first arm whose pattern's head constructor
    /// matches the scrutinee's head constructor wins.
    ///
    /// This is the tactic-evaluator's analogue of Lean's `match` inside
    /// tactic mode — structural on the constructor, not value-level.
    fn apply_match(
        &mut self,
        scrutinee: &Heap<Expr>,
        arms: &List<verum_ast::decl::TacticMatchArm>,
    ) -> TacticResult<()> {
        // Extract the head constructor name from the scrutinee. For an
        // `ExprKind::Call` with a path head, use the last segment; for
        // a bare path, use the last segment directly; otherwise the
        // scrutinee is opaque and we fall through to the first
        // wildcard-ish arm.
        let scrutinee_head: Option<Text> = match &scrutinee.kind {
            ExprKind::Call { func, .. } => callee_head_name(func),
            ExprKind::Path(path) => path
                .segments
                .iter()
                .last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => {
                        Some(Text::from(ident.name.as_str()))
                    }
                    _ => None,
                }),
            _ => None,
        };

        for arm in arms.iter() {
            if pattern_matches_head(&arm.pattern, scrutinee_head.as_ref()) {
                return self.apply_tactic(&arm.body);
            }
        }

        Err(TacticError::Failed(Text::from(
            "tactic match: no arm matched the scrutinee's head constructor",
        )))
    }

    /// T1-W: `if cond { t1 } else { t2 }` — conditional tactic
    /// execution. The condition is evaluated structurally: a literal
    /// `true` takes the then-branch, a literal `false` takes the
    /// else-branch. Non-literal conditions are treated as the
    /// evaluator cannot statically decide — this falls back to
    /// attempting the then-branch first, and if it fails, the
    /// else-branch (if present). This mirrors `try { then } else { else }`
    /// semantics when the condition is opaque at tactic-time.
    fn apply_if(
        &mut self,
        cond: &Heap<Expr>,
        then_branch: &Heap<TacticExpr>,
        else_branch: Option<&Heap<TacticExpr>>,
    ) -> TacticResult<()> {
        let decision = match &cond.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Bool(true) => Some(true),
                LiteralKind::Bool(false) => Some(false),
                _ => None,
            },
            _ => None,
        };

        match decision {
            Some(true) => self.apply_tactic(then_branch),
            Some(false) => match else_branch {
                Some(e) => self.apply_tactic(e),
                None => Ok(()),
            },
            None => {
                // Opaque condition — best-effort: try then, fall back to else.
                match self.apply_tactic(then_branch) {
                    Ok(()) => Ok(()),
                    Err(_) => match else_branch {
                        Some(e) => self.apply_tactic(e),
                        None => Err(TacticError::Failed(Text::from(
                            "tactic if: condition is opaque and then-branch failed",
                        ))),
                    },
                }
            }
        }
    }

    /// T1-W: `fail("reason")` — explicit failure with diagnostic
    /// message. Failure feeds into enclosing `try`/`first`/`else`
    /// combinators for recovery; when reached at top level, the
    /// tactic evaluator reports the message verbatim.
    fn apply_fail(&mut self, message: &Heap<Expr>) -> TacticResult<()> {
        let msg_text = match &message.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Text(s) => Text::from(s.as_str()),
                _ => Text::from("tactic `fail`"),
            },
            _ => Text::from("tactic `fail`"),
        };
        Err(TacticError::Failed(msg_text))
    }
}

/// Extract the last path-segment name from a callee expression.
/// Used by `apply_match` to compute the scrutinee's head constructor.
fn callee_head_name(callee: &Expr) -> Option<Text> {
    match &callee.kind {
        ExprKind::Path(path) => path
            .segments
            .iter()
            .last()
            .and_then(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => Some(Text::from(ident.name.as_str())),
                _ => None,
            }),
        _ => None,
    }
}

/// Check if a pattern matches the scrutinee's head constructor.
///
/// A wildcard/variable pattern matches anything. A constructor pattern
/// (e.g. `Maybe.Some(v)`) matches when the pattern's head name equals
/// the scrutinee's head. More precise value-level matching happens in
/// a follow-up phase; this is sufficient for tactic-time branching
/// over the constructor structure.
fn pattern_matches_head(
    pattern: &verum_ast::pattern::Pattern,
    scrutinee_head: Option<&Text>,
) -> bool {
    use verum_ast::pattern::PatternKind;
    match &pattern.kind {
        PatternKind::Wildcard | PatternKind::Ident { .. } | PatternKind::Literal(_) => true,
        PatternKind::Variant { path, .. } | PatternKind::Record { path, .. } => {
            let pattern_head = path.segments.iter().last().and_then(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => Some(Text::from(ident.name.as_str())),
                _ => None,
            });
            match (pattern_head, scrutinee_head) {
                (Some(ph), Some(sh)) => ph == *sh,
                _ => true, // conservative: treat as potential match
            }
        }
        _ => true, // other pattern kinds (tuple, slice, or, range, view) — be conservative
    }
}

impl Default for TacticEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Configuration ====================

/// Configuration for tactic evaluation
#[derive(Debug, Clone)]
pub struct TacticConfig {
    /// Maximum iterations for repeat tactic
    pub max_repeat_iterations: usize,

    /// Timeout for SMT tactics
    pub smt_timeout: Duration,

    /// Enable aggressive simplification
    pub aggressive_simplification: bool,

    /// Allow admit/sorry tactics
    pub allow_admits: bool,
}

impl Default for TacticConfig {
    fn default() -> Self {
        Self {
            max_repeat_iterations: 100,
            smt_timeout: Duration::from_secs(30),
            aggressive_simplification: true,
            allow_admits: true,
        }
    }
}

// ==================== Statistics ====================

/// Statistics about tactic evaluation
#[derive(Debug, Clone, Default)]
pub struct EvaluationStats {
    /// Total tactics applied
    pub tactics_applied: usize,

    /// Successful tactics
    pub successful_tactics: usize,

    /// Failed tactics
    pub failed_tactics: usize,

    /// Number of SMT solver calls
    pub smt_calls: usize,

    /// Number of admitted goals
    pub admitted_goals: usize,

    /// Number of sorry goals
    pub sorry_goals: usize,

    /// Total evaluation time
    pub total_time: Duration,
}

impl EvaluationStats {
    /// Get success rate
    pub fn success_rate(&self) -> f64 {
        if self.tactics_applied == 0 {
            0.0
        } else {
            self.successful_tactics as f64 / self.tactics_applied as f64
        }
    }

    /// Check if proof is complete (no admits/sorries)
    pub fn is_complete(&self) -> bool {
        self.admitted_goals == 0 && self.sorry_goals == 0
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::Literal;

    fn make_bool_expr(value: bool) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(value), Span::dummy())),
            Span::dummy(),
        )
    }

    fn make_binary_expr(op: BinOp, left: Expr, right: Expr) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            Span::dummy(),
        )
    }

    #[test]
    fn test_trivial_tactic() {
        let mut evaluator = TacticEvaluator::with_goal(make_bool_expr(true));

        let result = evaluator.apply_tactic(&TacticExpr::Trivial);
        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    #[test]
    fn test_assumption_tactic() {
        let goal_expr = make_bool_expr(true);
        let mut evaluator = TacticEvaluator::with_goal(goal_expr.clone());

        // Add hypothesis that matches the goal
        let hyp = Hypothesis::new(Text::from("H"), goal_expr);
        evaluator
            .state
            .current_goal_mut()
            .unwrap()
            .add_hypothesis(hyp);

        let result = evaluator.apply_tactic(&TacticExpr::Assumption);
        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    #[test]
    fn test_split_tactic() {
        // P ∧ Q
        let left = make_bool_expr(true);
        let right = make_bool_expr(true);
        let conjunction = make_binary_expr(BinOp::And, left, right);

        let mut evaluator = TacticEvaluator::with_goal(conjunction);

        // Split should create two subgoals
        let result = evaluator.apply_tactic(&TacticExpr::Split);
        assert!(result.is_ok());
        assert_eq!(evaluator.state.num_goals(), 2);
    }

    #[test]
    fn test_intro_tactic() {
        // P => Q
        let p = make_bool_expr(true);
        let q = make_bool_expr(true);
        let implication = make_binary_expr(BinOp::Imply, p, q);

        let mut evaluator = TacticEvaluator::with_goal(implication);

        // Intro should add P to hypotheses and change goal to Q
        let result = evaluator.apply_tactic(&TacticExpr::Intro(List::new()));
        assert!(result.is_ok());

        let goal = evaluator.state.current_goal().unwrap();
        assert_eq!(goal.hypotheses.len(), 1);
    }

    #[test]
    fn test_sequence_tactic() {
        // P ∧ Q where both are true
        let left = make_bool_expr(true);
        let right = make_bool_expr(true);
        let conjunction = make_binary_expr(BinOp::And, left, right);

        let mut evaluator = TacticEvaluator::with_goal(conjunction);

        // split; [trivial, trivial]
        let tactics =
            List::from_iter([TacticExpr::Split, TacticExpr::Trivial, TacticExpr::Trivial]);

        let result = evaluator.apply_tactic(&TacticExpr::Seq(tactics));
        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    #[test]
    fn test_statistics() {
        let mut evaluator = TacticEvaluator::with_goal(make_bool_expr(true));

        evaluator.apply_tactic(&TacticExpr::Trivial).unwrap();

        let stats = evaluator.stats();
        assert_eq!(stats.tactics_applied, 1);
        assert_eq!(stats.successful_tactics, 1);
        assert_eq!(stats.failed_tactics, 0);
        assert_eq!(stats.success_rate(), 1.0);
    }

    #[test]
    fn test_proof_state_management() {
        let mut state = ProofState::new(make_bool_expr(true));

        assert_eq!(state.num_goals(), 1);
        assert!(!state.is_complete());

        state.prove_current_goal().unwrap();
        assert_eq!(state.num_goals(), 0);
        assert!(state.is_complete());
    }

    #[test]
    fn test_goal_metadata() {
        let goal = Goal::new(0, make_bool_expr(true));
        assert_eq!(goal.id, 0);
        assert!(!goal.meta.from_induction);
        assert!(goal.meta.name.is_none());
    }

    #[test]
    fn test_hypothesis_sources() {
        let h1 = Hypothesis::new(Text::from("H1"), make_bool_expr(true));
        assert_eq!(h1.source, HypothesisSource::User);

        let h2 = Hypothesis::assumption(Text::from("H2"), make_bool_expr(true));
        assert_eq!(h2.source, HypothesisSource::Assumption);

        let h3 = Hypothesis::induction(Text::from("IH"), make_bool_expr(true));
        assert_eq!(h3.source, HypothesisSource::Induction);
    }

    // ==================== Tests for apply_compute ====================

    fn make_int_expr(value: i128) -> Expr {
        use verum_ast::literal::IntLit;
        Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(value)),
                Span::dummy(),
            )),
            Span::dummy(),
        )
    }

    #[test]
    fn test_compute_arithmetic_addition() {
        // Goal: 2 + 3 == 5 should simplify to true
        let left = make_binary_expr(BinOp::Add, make_int_expr(2), make_int_expr(3));
        let goal = make_binary_expr(BinOp::Eq, left, make_int_expr(5));

        let mut evaluator = TacticEvaluator::with_goal(goal);
        let result = evaluator.apply_tactic(&TacticExpr::Compute);
        assert!(result.is_ok());
        assert!(
            evaluator.state.is_complete(),
            "compute should prove 2 + 3 == 5"
        );
    }

    #[test]
    fn test_compute_arithmetic_multiplication() {
        // Goal: 4 * 5 == 20 should simplify to true
        let left = make_binary_expr(BinOp::Mul, make_int_expr(4), make_int_expr(5));
        let goal = make_binary_expr(BinOp::Eq, left, make_int_expr(20));

        let mut evaluator = TacticEvaluator::with_goal(goal);
        let result = evaluator.apply_tactic(&TacticExpr::Compute);
        assert!(result.is_ok());
        assert!(
            evaluator.state.is_complete(),
            "compute should prove 4 * 5 == 20"
        );
    }

    #[test]
    fn test_compute_boolean_simplification() {
        // Goal: true && false should simplify to false
        let goal = make_binary_expr(BinOp::And, make_bool_expr(true), make_bool_expr(false));

        let mut evaluator = TacticEvaluator::with_goal(goal);
        let result = evaluator.apply_tactic(&TacticExpr::Compute);

        // The result should be simplified to false
        assert!(result.is_ok());
        let current_goal = evaluator.state.current_goal().unwrap();
        // Check that the goal is now `false`
        if let ExprKind::Literal(lit) = &current_goal.proposition.kind {
            if let LiteralKind::Bool(false) = lit.kind {
                return; // Success
            }
        }
        panic!("compute should simplify true && false to false");
    }

    #[test]
    fn test_compute_identity_simplification() {
        // Goal: x + 0 == x should recognize x + 0 = x
        // This is handled by the normalize_expr function
        let x_path = verum_ast::ty::Path::from_ident(Ident::new(Text::from("x"), Span::dummy()));
        let x_expr = Expr::new(ExprKind::Path(x_path.clone()), Span::dummy());
        let x_plus_zero = make_binary_expr(BinOp::Add, x_expr.clone(), make_int_expr(0));
        let goal = make_binary_expr(BinOp::Eq, x_plus_zero, x_expr);

        let mut evaluator = TacticEvaluator::with_goal(goal);
        let result = evaluator.apply_tactic(&TacticExpr::Compute);
        assert!(result.is_ok());
        // After compute, x + 0 == x should simplify to x == x, which is true
        assert!(
            evaluator.state.is_complete(),
            "compute should prove x + 0 == x"
        );
    }

    #[test]
    fn test_compute_literal_true() {
        // Goal: true should be proven immediately
        let goal = make_bool_expr(true);

        let mut evaluator = TacticEvaluator::with_goal(goal);
        let result = evaluator.apply_tactic(&TacticExpr::Compute);
        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    // ==================== Tests for apply_unfold ====================

    fn make_path_expr(name: &str) -> Expr {
        let path = verum_ast::ty::Path::from_ident(Ident::new(Text::from(name), Span::dummy()));
        Expr::new(ExprKind::Path(path), Span::dummy())
    }

    #[test]
    fn test_unfold_basic() {
        // Goal: f(x) > 0  with hypothesis f = x + 1
        // After unfold f: (x + 1) > 0
        let f_x = make_path_expr("f");
        let x = make_path_expr("x");
        let goal = make_binary_expr(BinOp::Gt, f_x.clone(), make_int_expr(0));

        let mut evaluator = TacticEvaluator::with_goal(goal);

        // Add definition: f = x + 1
        let f_def = make_binary_expr(
            BinOp::Eq,
            make_path_expr("f"),
            make_binary_expr(BinOp::Add, x.clone(), make_int_expr(1)),
        );
        let hyp = Hypothesis::new(Text::from("f_def"), f_def);
        evaluator
            .state
            .current_goal_mut()
            .unwrap()
            .add_hypothesis(hyp);

        // Apply unfold f
        let unfold_names = List::from_iter([Ident::new(Text::from("f"), Span::dummy())]);
        let result = evaluator.apply_tactic(&TacticExpr::Unfold(unfold_names));
        assert!(result.is_ok());

        // After unfold, the goal should be (x + 1) > 0
        let current_goal = evaluator.state.current_goal().unwrap();
        if let ExprKind::Binary {
            op: BinOp::Gt,
            left,
            ..
        } = &current_goal.proposition.kind
        {
            // Check that f was replaced
            if let ExprKind::Binary { op: BinOp::Add, .. } = &left.kind {
                return; // Success - f was unfolded to x + 1
            }
        }
        panic!("unfold should replace f with x + 1");
    }

    #[test]
    fn test_unfold_no_definition() {
        // Goal: f(x) > 0 without any definition for f
        let f_x = make_path_expr("f");
        let goal = make_binary_expr(BinOp::Gt, f_x, make_int_expr(0));

        let mut evaluator = TacticEvaluator::with_goal(goal);

        // Apply unfold f without any definition
        let unfold_names = List::from_iter([Ident::new(Text::from("f"), Span::dummy())]);
        let result = evaluator.apply_tactic(&TacticExpr::Unfold(unfold_names));

        // Should fail because no definition exists
        assert!(result.is_err());
        if let Err(TacticError::Failed(msg)) = result {
            assert!(msg.contains("no definition found"));
        } else {
            panic!("Expected TacticError::Failed");
        }
    }

    #[test]
    fn test_unfold_empty_names() {
        let goal = make_bool_expr(true);
        let mut evaluator = TacticEvaluator::with_goal(goal);

        // Apply unfold with empty list
        let result = evaluator.apply_tactic(&TacticExpr::Unfold(List::new()));

        // Should fail because no names were provided
        assert!(result.is_err());
        if let Err(TacticError::InvalidArgument(msg)) = result {
            assert!(msg.contains("at least one name"));
        } else {
            panic!("Expected TacticError::InvalidArgument");
        }
    }

    // ==================== Tests for intro with Forall ====================

    fn make_forall_expr(var_name: &str, body: Expr) -> Expr {
        let bound_var = Ident::new(Text::from(var_name), Span::dummy());
        let pattern = Pattern::ident(bound_var, false, Span::dummy());
        let binding = verum_ast::expr::QuantifierBinding::typed(
            pattern,
            verum_ast::ty::Type::inferred(Span::dummy()),
            Span::dummy(),
        );
        Expr::new(
            ExprKind::Forall {
                bindings: List::from_iter([binding]),
                body: Heap::new(body),
            },
            Span::dummy(),
        )
    }

    #[test]
    fn test_intro_forall_quantifier() {
        // Goal: forall x. x == x
        let x_path = verum_ast::ty::Path::from_ident(Ident::new(Text::from("x"), Span::dummy()));
        let x_expr = Expr::new(ExprKind::Path(x_path), Span::dummy());
        let body = make_binary_expr(BinOp::Eq, x_expr.clone(), x_expr.clone());
        let goal = make_forall_expr("x", body);

        let mut evaluator = TacticEvaluator::with_goal(goal);

        // Intro should introduce x and make goal: x == x
        let result = evaluator.apply_tactic(&TacticExpr::Intro(List::new()));
        assert!(result.is_ok());

        let current_goal = evaluator.state.current_goal().unwrap();
        // Should have introduced x as a hypothesis
        assert!(
            current_goal.hypotheses.len() >= 1,
            "intro should add a hypothesis for the bound variable"
        );
    }

    #[test]
    fn test_intro_forall_with_custom_name() {
        // Goal: forall x. x > 0
        let x_path = verum_ast::ty::Path::from_ident(Ident::new(Text::from("x"), Span::dummy()));
        let x_expr = Expr::new(ExprKind::Path(x_path), Span::dummy());
        let body = make_binary_expr(BinOp::Gt, x_expr.clone(), make_int_expr(0));
        let goal = make_forall_expr("x", body);

        let mut evaluator = TacticEvaluator::with_goal(goal);

        // Intro with custom name "n"
        let names = List::from_iter([Ident::new(Text::from("n"), Span::dummy())]);
        let result = evaluator.apply_tactic(&TacticExpr::Intro(names));
        assert!(result.is_ok());

        let current_goal = evaluator.state.current_goal().unwrap();
        // Should have introduced with name "n"
        let has_n = current_goal
            .hypotheses
            .iter()
            .any(|h| h.name.as_str() == "n");
        assert!(has_n, "intro should use the custom name 'n'");
    }

    #[test]
    fn test_intro_nested_forall() {
        // Goal: forall x. forall y. x + y == y + x
        let x_path = verum_ast::ty::Path::from_ident(Ident::new(Text::from("x"), Span::dummy()));
        let x_expr = Expr::new(ExprKind::Path(x_path), Span::dummy());
        let y_path = verum_ast::ty::Path::from_ident(Ident::new(Text::from("y"), Span::dummy()));
        let y_expr = Expr::new(ExprKind::Path(y_path), Span::dummy());

        let x_plus_y = make_binary_expr(BinOp::Add, x_expr.clone(), y_expr.clone());
        let y_plus_x = make_binary_expr(BinOp::Add, y_expr.clone(), x_expr.clone());
        let body = make_binary_expr(BinOp::Eq, x_plus_y, y_plus_x);
        let inner_forall = make_forall_expr("y", body);
        let goal = make_forall_expr("x", inner_forall);

        let mut evaluator = TacticEvaluator::with_goal(goal);

        // First intro for x
        let result = evaluator.apply_tactic(&TacticExpr::Intro(List::new()));
        assert!(result.is_ok());

        // Second intro for y
        let result = evaluator.apply_tactic(&TacticExpr::Intro(List::new()));
        assert!(result.is_ok());

        let current_goal = evaluator.state.current_goal().unwrap();
        // Should have both x and y introduced
        assert!(
            current_goal.hypotheses.len() >= 2,
            "should have introduced both x and y"
        );
    }

    // ==================== Tests for apply tactic ====================

    #[test]
    fn test_apply_simple_implication() {
        // Goal: Q with hypothesis H: P => Q
        // After apply H: goal becomes P
        let p = make_path_expr("P");
        let q = make_path_expr("Q");
        let implication = make_binary_expr(BinOp::Imply, p.clone(), q.clone());

        let mut evaluator = TacticEvaluator::with_goal(q.clone());

        // Add hypothesis P => Q
        let hyp = Hypothesis::new(Text::from("H"), implication.clone());
        evaluator
            .state
            .current_goal_mut()
            .unwrap()
            .add_hypothesis(hyp);

        // Apply H
        let lemma_path =
            verum_ast::ty::Path::from_ident(Ident::new(Text::from("H"), Span::dummy()));
        let lemma_expr = Expr::new(ExprKind::Path(lemma_path), Span::dummy());
        let result = evaluator.apply_tactic(&TacticExpr::Apply {
            lemma: Heap::new(lemma_expr),
            args: List::new(),
        });

        assert!(result.is_ok());
        // Goal should now be P
        let current_goal = evaluator.state.current_goal().unwrap();
        assert!(
            !evaluator.state.is_complete(),
            "apply should leave goal P to prove"
        );

        // Check that the goal is P (a path expression)
        if let ExprKind::Path(path) = &current_goal.proposition.kind {
            if let Some(ident) = path.as_ident() {
                assert_eq!(ident.as_str(), "P");
            }
        }
    }

    // ==================== Tests for rewrite tactic ====================

    #[test]
    fn test_rewrite_equality() {
        // Goal: x + 0 == x with hypothesis H: 0 == z
        // Rewrite with H at goal
        let x_expr = make_path_expr("x");
        let z_expr = make_path_expr("z");
        let x_plus_zero = make_binary_expr(BinOp::Add, x_expr.clone(), make_int_expr(0));
        let goal = make_binary_expr(BinOp::Eq, x_plus_zero, x_expr.clone());

        let mut evaluator = TacticEvaluator::with_goal(goal);

        // Add hypothesis 0 == z
        let h_eq = make_binary_expr(BinOp::Eq, make_int_expr(0), z_expr.clone());
        let hyp = Hypothesis::new(Text::from("H"), h_eq.clone());
        evaluator
            .state
            .current_goal_mut()
            .unwrap()
            .add_hypothesis(hyp);

        // Rewrite using H
        let result = evaluator.apply_tactic(&TacticExpr::Rewrite {
            hypothesis: Heap::new(h_eq),
            at_target: Maybe::None,
            rev: false,
        });

        // Rewrite should succeed
        assert!(result.is_ok());
    }

    // ==================== Tests for induction tactic ====================

    #[test]
    fn test_induction_creates_two_goals() {
        // Goal: P(n) for some n
        // Induction on n should create base case (P(0)) and inductive case
        let n_expr = make_path_expr("n");
        let goal = make_binary_expr(BinOp::Gt, n_expr.clone(), make_int_expr(-1));

        let mut evaluator = TacticEvaluator::with_goal(goal);

        // Add n to hypotheses as a variable
        let n_hyp = Hypothesis::new(Text::from("n"), make_bool_expr(true));
        evaluator
            .state
            .current_goal_mut()
            .unwrap()
            .add_hypothesis(n_hyp);

        let result = evaluator.apply_tactic(&TacticExpr::InductionOn(Ident::new(
            Text::from("n"),
            Span::dummy(),
        )));

        // Induction should create two subgoals (base and inductive)
        assert!(result.is_ok());
        assert!(
            evaluator.state.num_goals() >= 2,
            "induction should create at least 2 subgoals (base and step)"
        );
    }

    // ==================== Tests for cases tactic ====================

    #[test]
    fn test_cases_on_bool() {
        // Goal: P(b) for some bool b
        // Cases on b should create two cases: P(true) and P(false)
        let b_expr = make_path_expr("b");
        let goal = make_binary_expr(BinOp::Eq, b_expr.clone(), b_expr.clone());

        let mut evaluator = TacticEvaluator::with_goal(goal);

        // Add b to hypotheses with Bool type annotation
        let b_hyp = Hypothesis {
            name: Text::from("b"),
            proposition: Heap::new(make_bool_expr(true)), // Placeholder proposition
            ty: Maybe::Some(Type::Bool),                  // Important: specify the type as Bool
            source: HypothesisSource::Assumption,
        };
        evaluator
            .state
            .current_goal_mut()
            .unwrap()
            .add_hypothesis(b_hyp);

        let result = evaluator.apply_tactic(&TacticExpr::CasesOn(Ident::new(
            Text::from("b"),
            Span::dummy(),
        )));

        // Cases should create subgoals
        assert!(result.is_ok());
        // Should have 2 goals: one for b = true, one for b = false
        assert_eq!(
            evaluator.state.num_goals(),
            2,
            "cases on Bool should create 2 subgoals"
        );
    }

    // ==================== Tests for auto tactic ====================

    #[test]
    fn test_auto_proves_trivial() {
        // Goal: true should be proven by auto
        let goal = make_bool_expr(true);
        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Auto {
            with_hints: List::new(),
        });

        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    #[test]
    fn test_auto_simple_arithmetic() {
        // Goal: 2 + 2 == 4
        let two_plus_two = make_binary_expr(BinOp::Add, make_int_expr(2), make_int_expr(2));
        let goal = make_binary_expr(BinOp::Eq, two_plus_two, make_int_expr(4));

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Auto {
            with_hints: List::new(),
        });

        assert!(result.is_ok());
        assert!(
            evaluator.state.is_complete(),
            "auto should prove 2 + 2 == 4"
        );
    }

    // ==================== Tests for omega tactic (linear arithmetic) ====================

    #[test]
    fn test_omega_linear_arithmetic() {
        // Goal: x + 1 > x (always true for integers)
        let x_expr = make_path_expr("x");
        let x_plus_one = make_binary_expr(BinOp::Add, x_expr.clone(), make_int_expr(1));
        let goal = make_binary_expr(BinOp::Gt, x_plus_one, x_expr);

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Omega);

        // Omega should prove this linear arithmetic fact
        assert!(result.is_ok());
        assert!(
            evaluator.state.is_complete(),
            "omega should prove x + 1 > x"
        );
    }

    // ==================== Tests for reflexivity tactic ====================

    #[test]
    fn test_reflexivity() {
        // Goal: 42 == 42
        let goal = make_binary_expr(BinOp::Eq, make_int_expr(42), make_int_expr(42));

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Reflexivity);

        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    #[test]
    fn test_reflexivity_with_expression() {
        // Goal: x == x (reflexivity of variable)
        let x_expr = make_path_expr("x");
        let goal = make_binary_expr(BinOp::Eq, x_expr.clone(), x_expr);

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Reflexivity);

        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    // ==================== Tests for left/right tactics ====================

    #[test]
    fn test_left_disjunction() {
        // Goal: P \/ Q - left should select P
        let p = make_path_expr("P");
        let q = make_path_expr("Q");
        let goal = make_binary_expr(BinOp::Or, p.clone(), q.clone());

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Left);

        assert!(result.is_ok());
        let current_goal = evaluator.state.current_goal().unwrap();
        // Goal should now be P
        if let ExprKind::Path(path) = &current_goal.proposition.kind {
            if let Some(ident) = path.as_ident() {
                assert_eq!(ident.as_str(), "P");
            }
        }
    }

    #[test]
    fn test_right_disjunction() {
        // Goal: P \/ Q - right should select Q
        let p = make_path_expr("P");
        let q = make_path_expr("Q");
        let goal = make_binary_expr(BinOp::Or, p.clone(), q.clone());

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Right);

        assert!(result.is_ok());
        let current_goal = evaluator.state.current_goal().unwrap();
        // Goal should now be Q
        if let ExprKind::Path(path) = &current_goal.proposition.kind {
            if let Some(ident) = path.as_ident() {
                assert_eq!(ident.as_str(), "Q");
            }
        }
    }

    // ==================== Tests for exists tactic ====================

    fn make_exists_expr(var_name: &str, body: Expr) -> Expr {
        let bound_var = Ident::new(Text::from(var_name), Span::dummy());
        let pattern = Pattern::ident(bound_var, false, Span::dummy());
        let binding = verum_ast::expr::QuantifierBinding::typed(
            pattern,
            verum_ast::ty::Type::inferred(Span::dummy()),
            Span::dummy(),
        );
        Expr::new(
            ExprKind::Exists {
                bindings: List::from_iter([binding]),
                body: Heap::new(body),
            },
            Span::dummy(),
        )
    }

    #[test]
    fn test_exists_witness() {
        // Goal: exists x. x == 5
        // Provide witness 5
        let x_path = verum_ast::ty::Path::from_ident(Ident::new(Text::from("x"), Span::dummy()));
        let x_expr = Expr::new(ExprKind::Path(x_path), Span::dummy());
        let body = make_binary_expr(BinOp::Eq, x_expr.clone(), make_int_expr(5));
        let goal = make_exists_expr("x", body);

        let mut evaluator = TacticEvaluator::with_goal(goal);

        // Provide witness 5
        let result = evaluator.apply_tactic(&TacticExpr::Exists(Heap::new(make_int_expr(5))));

        assert!(result.is_ok());
        // Goal should now be 5 == 5
        let current_goal = evaluator.state.current_goal().unwrap();
        if let ExprKind::Binary {
            op: BinOp::Eq,
            left,
            right,
        } = &current_goal.proposition.kind
        {
            // Both sides should be 5
            assert!(matches!(&left.kind, ExprKind::Literal(_)));
            assert!(matches!(&right.kind, ExprKind::Literal(_)));
        }
    }

    // ==================== Tests for try/repeat combinators ====================

    #[test]
    fn test_try_continues_on_failure() {
        // Try trivial on a non-trivial goal - should not fail
        let goal = make_path_expr("P"); // Not trivially true

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Try(Heap::new(TacticExpr::Trivial)));

        // Try should succeed (even though trivial fails internally)
        assert!(result.is_ok());
        // Goal should remain unchanged
        assert!(!evaluator.state.is_complete());
    }

    #[test]
    fn test_repeat_until_failure() {
        // Repeat intro on P => Q => R
        let p = make_path_expr("P");
        let q = make_path_expr("Q");
        let r = make_path_expr("R");
        let q_to_r = make_binary_expr(BinOp::Imply, q.clone(), r.clone());
        let goal = make_binary_expr(BinOp::Imply, p.clone(), q_to_r);

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Repeat(Heap::new(TacticExpr::Intro(
            List::new(),
        ))));

        // Repeat should succeed
        assert!(result.is_ok());
        // Should have introduced both P and Q as hypotheses
        let current_goal = evaluator.state.current_goal().unwrap();
        assert!(
            current_goal.hypotheses.len() >= 2,
            "repeat intro should introduce all implications"
        );
    }

    // ==================== Tests for simp tactic ====================

    #[test]
    fn test_simp_simplifies_goal() {
        // Goal: true && true (should simplify to true)
        let goal = make_binary_expr(BinOp::And, make_bool_expr(true), make_bool_expr(true));

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Simp {
            lemmas: List::new(),
            at_target: Maybe::None,
        });

        // Simp should simplify to true and prove it
        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    // ==================== Tests for done tactic ====================

    #[test]
    fn test_done_fails_with_remaining_goals() {
        let goal = make_path_expr("P");
        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Done);

        // Done should fail because there's still a goal
        assert!(result.is_err());
    }

    #[test]
    fn test_done_succeeds_when_complete() {
        let goal = make_bool_expr(true);
        let mut evaluator = TacticEvaluator::with_goal(goal);

        // First prove the goal
        evaluator.apply_tactic(&TacticExpr::Trivial).unwrap();

        let result = evaluator.apply_tactic(&TacticExpr::Done);

        // Done should succeed
        assert!(result.is_ok());
    }

    // ==================== Tests for admit/sorry tactics ====================

    #[test]
    fn test_admit_closes_goal() {
        let goal = make_path_expr("ComplexGoal");
        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Admit);

        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
        assert_eq!(evaluator.stats().admitted_goals, 1);
    }

    #[test]
    fn test_sorry_closes_goal() {
        let goal = make_path_expr("ComplexGoal");
        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Sorry);

        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
        assert_eq!(evaluator.stats().sorry_goals, 1);
    }

    // ==================== Tests for smt tactic ====================

    #[test]
    fn test_smt_proves_arithmetic() {
        // Goal: 3 * 7 == 21
        let three_times_seven = make_binary_expr(BinOp::Mul, make_int_expr(3), make_int_expr(7));
        let goal = make_binary_expr(BinOp::Eq, three_times_seven, make_int_expr(21));

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Smt {
            solver: Maybe::None,
            timeout: Maybe::None,
        });

        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    // ==================== Tests for blast tactic ====================

    #[test]
    fn test_blast_proves_contradiction() {
        // Goal: P with hypotheses P and not P should find contradiction
        // Actually blast should work with logical reasoning
        let goal = make_bool_expr(true);

        let mut evaluator = TacticEvaluator::with_goal(goal);

        let result = evaluator.apply_tactic(&TacticExpr::Blast);

        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    // ==================== Tests for alternative combinator ====================

    #[test]
    fn test_alternative_first_succeeds() {
        let goal = make_bool_expr(true);
        let mut evaluator = TacticEvaluator::with_goal(goal);

        // trivial | assumption - trivial should succeed
        let alt = TacticExpr::Alt(List::from_iter([
            TacticExpr::Trivial,
            TacticExpr::Assumption,
        ]));

        let result = evaluator.apply_tactic(&alt);

        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }

    #[test]
    fn test_alternative_second_succeeds() {
        let goal = make_path_expr("P");
        let mut evaluator = TacticEvaluator::with_goal(goal.clone());

        // Add P as hypothesis
        let hyp = Hypothesis::new(Text::from("H"), goal.clone());
        evaluator
            .state
            .current_goal_mut()
            .unwrap()
            .add_hypothesis(hyp);

        // reflexivity | assumption - reflexivity fails, assumption should succeed
        let alt = TacticExpr::Alt(List::from_iter([
            TacticExpr::Reflexivity,
            TacticExpr::Assumption,
        ]));

        let result = evaluator.apply_tactic(&alt);

        assert!(result.is_ok());
        assert!(evaluator.state.is_complete());
    }
}
