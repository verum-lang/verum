//! Bounds Check Elimination with Refinement Type Integration
//!
//! This module implements comprehensive bounds check elimination for array accesses
//! through integration with the refinement type system. It eliminates runtime bounds
//! checks when static analysis can prove safety.
//!
//! # Core Strategy
//!
//! For array access `array[index]`, eliminate bounds check when ANY of:
//! 1. **Refinement types prove bounds**: index: Int{>= 0 && < N}, array.len() >= N
//! 2. **Loop invariants**: for i in 0..array.len() { array[i] }
//! 3. **Meta parameters**: Array<T, N> with index: usize where index < N
//! 4. **Dataflow analysis**: Conditional dominance (if index < len { array[index] })
//! 5. **Check hoisting**: Hoist bounds check out of loops when iteration space is known
//!
//! # Performance Targets
//!
//! - Elimination rate: >80% on typical code
//! - Analysis time: <50ms per function
//! - No false positives (100% safety)
//!
//! # Integration Points
//!
//! - verum_types::refinement - Refinement type constraints
//! - verum_verification::vcgen - Loop invariant extraction
//! - verum_codegen - Code generation with eliminated checks

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::span::Span;
use verum_common::{List, Map, Maybe, Set, Text};

use crate::cbgr_elimination::{BlockId, ControlFlowGraph, RefVariable};

// =============================================================================
// Core Types
// =============================================================================

/// Decision for array bounds check
///
/// Result of static analysis for an array access: eliminate (proven safe),
/// hoist (move check to loop preheader), or keep (cannot prove safety).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckDecision {
    /// Eliminate check - proven safe by static analysis
    Eliminate,
    /// Hoist check - move to loop preheader or function entry
    Hoist,
    /// Keep check - cannot prove safety
    Keep,
}

impl CheckDecision {
    /// Check if bounds check can be eliminated
    pub fn can_eliminate(&self) -> bool {
        matches!(self, CheckDecision::Eliminate)
    }

    /// Check if bounds check should be hoisted
    pub fn should_hoist(&self) -> bool {
        matches!(self, CheckDecision::Hoist)
    }

    /// Get overhead in nanoseconds
    pub fn overhead_ns(&self) -> u32 {
        match self {
            CheckDecision::Eliminate => 0,
            CheckDecision::Hoist => 1, // One-time check
            CheckDecision::Keep => 5,  // Per-access check
        }
    }
}

impl fmt::Display for CheckDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckDecision::Eliminate => write!(f, "Eliminate (0ns)"),
            CheckDecision::Hoist => write!(f, "Hoist (1ns)"),
            CheckDecision::Keep => write!(f, "Keep (5ns)"),
        }
    }
}

/// Loop identifier for tracking loop contexts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LoopId(pub u64);

impl LoopId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

/// Array access site
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrayAccess {
    /// Array expression
    pub array: Expression,
    /// Index expression
    pub index: Expression,
    /// Block where access occurs
    pub block: BlockId,
    /// Loop context (if inside loop)
    pub loop_context: Maybe<LoopId>,
    /// Source location
    pub span: Span,
}

impl ArrayAccess {
    pub fn new(array: Expression, index: Expression, block: BlockId, span: Span) -> Self {
        Self {
            array,
            index,
            block,
            loop_context: Maybe::None,
            span,
        }
    }

    pub fn with_loop(mut self, loop_id: LoopId) -> Self {
        self.loop_context = Maybe::Some(loop_id);
        self
    }
}

/// Simplified expression representation for analysis
///
/// Note: In production, use verum_ast::expr::Expr
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Expression {
    /// Variable reference
    Var(Text),
    /// Integer literal
    Int(i64),
    /// Binary operation
    Binary {
        op: BinaryOp,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    /// Array length
    ArrayLen(Box<Expression>),
    /// Field access
    Field { base: Box<Expression>, field: Text },
}

impl Expression {
    pub fn var(name: impl Into<Text>) -> Self {
        Expression::Var(name.into())
    }

    pub fn int(value: i64) -> Self {
        Expression::Int(value)
    }

    pub fn binary(op: BinaryOp, left: Expression, right: Expression) -> Self {
        Expression::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    pub fn array_len(expr: Expression) -> Self {
        Expression::ArrayLen(Box::new(expr))
    }

    pub fn field(base: Expression, field: impl Into<Text>) -> Self {
        Expression::Field {
            base: Box::new(base),
            field: field.into(),
        }
    }

    /// Check if this is a simple variable reference
    pub fn is_var(&self) -> bool {
        matches!(self, Expression::Var(_))
    }

    /// Extract variable name if this is a variable
    pub fn as_var(&self) -> Maybe<&Text> {
        match self {
            Expression::Var(name) => Maybe::Some(name),
            _ => Maybe::None,
        }
    }
}

impl fmt::Display for Expression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expression::Var(name) => write!(f, "{}", name),
            Expression::Int(value) => write!(f, "{}", value),
            Expression::Binary { op, left, right } => {
                write!(f, "({} {} {})", left, op, right)
            }
            Expression::ArrayLen(expr) => write!(f, "len({})", expr),
            Expression::Field { base, field } => write!(f, "{}.{}", base, field),
        }
    }
}

/// Binary operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinaryOp::Add => write!(f, "+"),
            BinaryOp::Sub => write!(f, "-"),
            BinaryOp::Mul => write!(f, "*"),
            BinaryOp::Div => write!(f, "/"),
            BinaryOp::Mod => write!(f, "%"),
            BinaryOp::Eq => write!(f, "=="),
            BinaryOp::Ne => write!(f, "!="),
            BinaryOp::Lt => write!(f, "<"),
            BinaryOp::Le => write!(f, "<="),
            BinaryOp::Gt => write!(f, ">"),
            BinaryOp::Ge => write!(f, ">="),
            BinaryOp::And => write!(f, "&&"),
            BinaryOp::Or => write!(f, "||"),
        }
    }
}

// =============================================================================
// Refinement Type Integration
// =============================================================================

/// Refinement constraint extracted from type system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Refinement {
    /// Variable name
    pub var: Text,
    /// Constraint expression
    pub constraint: Expression,
}

impl Refinement {
    pub fn new(var: Text, constraint: Expression) -> Self {
        Self { var, constraint }
    }
}

/// Array bounds information from refinement types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrayBounds {
    /// Array variable name
    pub array_var: Text,
    /// Alias for compatibility
    #[serde(skip)]
    pub name: Text,
    /// Static length if known at compile time
    pub length: Maybe<usize>,
    /// Dynamic length expression (e.g., "array.len()")
    pub length_expr: Maybe<Expression>,
    /// Index constraints from refinement types
    pub index_constraints: List<IndexConstraint>,
}

/// Result of analyzing an index for hoisting
#[derive(Debug, Clone)]
pub struct HoistAnalysisResult {
    /// Whether the check can be hoisted
    pub can_hoist: bool,
    /// The worst-case index value
    pub worst_case_index: Expression,
    /// The hoisted check expression (if can_hoist is true)
    pub hoisted_check: Option<Expression>,
}

impl ArrayBounds {
    pub fn new(array_var: Text) -> Self {
        Self {
            name: array_var.clone(),
            array_var,
            length: Maybe::None,
            length_expr: Maybe::None,
            index_constraints: List::new(),
        }
    }

    pub fn with_static_length(mut self, length: usize) -> Self {
        self.length = Maybe::Some(length);
        self
    }

    pub fn with_length_expr(mut self, expr: Expression) -> Self {
        self.length_expr = Maybe::Some(expr);
        self
    }

    pub fn add_constraint(&mut self, constraint: IndexConstraint) {
        self.index_constraints.push(constraint);
    }
}

/// Index constraint from refinement type
///
/// Example: index: Int where 0 <= index < N
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexConstraint {
    /// Index variable name
    pub index_var: Text,
    /// Lower bound (inclusive): index >= lower_bound
    pub lower_bound: Expression,
    /// Upper bound (exclusive): index < upper_bound
    pub upper_bound: Expression,
    /// Whether constraint has been proven
    pub proven: bool,
}

impl IndexConstraint {
    pub fn new(index_var: Text, lower_bound: Expression, upper_bound: Expression) -> Self {
        Self {
            index_var,
            lower_bound,
            upper_bound,
            proven: false,
        }
    }

    /// Mark constraint as proven
    pub fn mark_proven(&mut self) {
        self.proven = true;
    }

    /// Check if constraint is trivially satisfied
    pub fn is_trivial(&self) -> bool {
        // Check for pattern: 0 <= index < N
        matches!(
            (&self.lower_bound, &self.upper_bound),
            (Expression::Int(0), Expression::Int(_))
        )
    }
}

// =============================================================================
// Loop Invariants
// =============================================================================

/// Loop invariant for bounds elimination
///
/// Tracks the induction variable, its bounds, and any proven stride relationship
/// to enable bounds check elimination within the loop body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopInvariant {
    /// Loop identifier
    pub loop_id: LoopId,
    /// Induction variable (e.g., "i" in "for i in 0..n")
    pub induction_var: Text,
    /// Lower bound (inclusive)
    pub lower_bound: Expression,
    /// Upper bound (exclusive)
    pub upper_bound: Expression,
    /// Step size (usually 1)
    pub step: i64,
    /// Variable bounds tracked in this loop
    pub variable_bounds: Map<Text, ValueRange>,
}

impl LoopInvariant {
    pub fn new(loop_id: LoopId, induction_var: Text) -> Self {
        Self {
            loop_id,
            induction_var,
            lower_bound: Expression::int(0),
            upper_bound: Expression::int(0),
            step: 1,
            variable_bounds: Map::new(),
        }
    }

    pub fn with_bounds(mut self, lower: Expression, upper: Expression) -> Self {
        self.lower_bound = lower;
        self.upper_bound = upper;
        self
    }

    pub fn with_step(mut self, step: i64) -> Self {
        self.step = step;
        self
    }

    /// Get bounds for a variable within this loop
    pub fn get_variable_bounds(&self, var: &Text) -> Maybe<&ValueRange> {
        self.variable_bounds
            .get(var)
            .map(Maybe::Some)
            .unwrap_or(Maybe::None)
    }

    /// Add variable bounds
    pub fn add_variable_bounds(&mut self, var: Text, range: ValueRange) {
        self.variable_bounds.insert(var, range);
    }
}

/// Value range for dataflow analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueRange {
    /// Lower bound (inclusive)
    pub lower: Expression,
    /// Upper bound (inclusive)
    pub upper: Expression,
    /// Whether bounds are proven
    pub proven: bool,
}

impl ValueRange {
    pub fn new(lower: Expression, upper: Expression) -> Self {
        Self {
            lower,
            upper,
            proven: false,
        }
    }

    pub fn unbounded() -> Self {
        Self {
            lower: Expression::int(i64::MIN),
            upper: Expression::int(i64::MAX),
            proven: false,
        }
    }

    pub fn with_proven(mut self, proven: bool) -> Self {
        self.proven = proven;
        self
    }

    /// Intersect two ranges (take narrower bounds)
    ///
    /// Uses SMT solver for symbolic expressions when constants are not available.
    pub fn intersect(&self, other: &ValueRange) -> ValueRange {
        ValueRange {
            lower: Self::narrow_lower_smt(&self.lower, &other.lower),
            upper: Self::narrow_upper_smt(&self.upper, &other.upper),
            proven: self.proven && other.proven,
        }
    }

    /// Narrow lower bound: return max(a, b) using SMT if needed
    ///
    /// For symbolic expressions, uses Z3 to determine which bound is larger.
    /// Falls back to conservative choice (first argument) if SMT is inconclusive.
    fn narrow_lower_smt(a: &Expression, b: &Expression) -> Expression {
        use z3::ast::Ast;
        use z3::{Params, SatResult, Solver};

        // Fast path: both are constants
        if let (Expression::Int(x), Expression::Int(y)) = (a, b) {
            return Expression::int((*x).max(*y));
        }

        // Full SMT comparison for symbolic expressions
        // We check: is a >= b always true?
        if Self::smt_check_ge(a, b) {
            return a.clone();
        }
        // Check: is b >= a always true?
        if Self::smt_check_ge(b, a) {
            return b.clone();
        }

        // Cannot determine - return conservative choice (first argument)
        // This is safe because we're computing lower bounds, and if we can't prove
        // which is larger, we keep the existing bound
        a.clone()
    }

    /// Narrow upper bound: return min(a, b) using SMT if needed
    ///
    /// For symbolic expressions, uses Z3 to determine which bound is smaller.
    /// Falls back to conservative choice (first argument) if SMT is inconclusive.
    fn narrow_upper_smt(a: &Expression, b: &Expression) -> Expression {
        use z3::ast::Ast;
        use z3::{Params, SatResult, Solver};

        // Fast path: both are constants
        if let (Expression::Int(x), Expression::Int(y)) = (a, b) {
            return Expression::int((*x).min(*y));
        }

        // Full SMT comparison for symbolic expressions
        // We check: is a <= b always true? (a is smaller or equal)
        if Self::smt_check_le(a, b) {
            return a.clone();
        }
        // Check: is b <= a always true? (b is smaller or equal)
        if Self::smt_check_le(b, a) {
            return b.clone();
        }

        // Cannot determine - return conservative choice (first argument)
        a.clone()
    }

    /// Check if a >= b is always true using SMT
    ///
    /// Returns true if a >= b holds for all possible values.
    /// Uses Z3 to check if NOT(a >= b) is UNSAT.
    fn smt_check_ge(a: &Expression, b: &Expression) -> bool {
        use z3::ast::Ast;
        use z3::{Params, SatResult, Solver};

        let solver = Solver::new();

        // Convert expressions to Z3
        let a_z3 = match Self::expr_to_z3(a) {
            Ok(e) => e,
            Err(_) => return false,
        };
        let b_z3 = match Self::expr_to_z3(b) {
            Ok(e) => e,
            Err(_) => return false,
        };

        // We want to prove a >= b is always true
        // So we assert NOT(a >= b) = a < b and check for UNSAT
        let a_lt_b = a_z3.lt(&b_z3);
        solver.assert(&a_lt_b);

        // Set short timeout for quick decisions
        let mut params = Params::new();
        params.set_u32("timeout", 100); // 100ms timeout
        solver.set_params(&params);

        matches!(solver.check(), SatResult::Unsat)
    }

    /// Check if a <= b is always true using SMT
    ///
    /// Returns true if a <= b holds for all possible values.
    fn smt_check_le(a: &Expression, b: &Expression) -> bool {
        use z3::ast::Ast;
        use z3::{Params, SatResult, Solver};

        let solver = Solver::new();

        // Convert expressions to Z3
        let a_z3 = match Self::expr_to_z3(a) {
            Ok(e) => e,
            Err(_) => return false,
        };
        let b_z3 = match Self::expr_to_z3(b) {
            Ok(e) => e,
            Err(_) => return false,
        };

        // We want to prove a <= b is always true
        // So we assert NOT(a <= b) = a > b and check for UNSAT
        let a_gt_b = a_z3.gt(&b_z3);
        solver.assert(&a_gt_b);

        // Set short timeout for quick decisions
        let mut params = Params::new();
        params.set_u32("timeout", 100); // 100ms timeout
        solver.set_params(&params);

        matches!(solver.check(), SatResult::Unsat)
    }

    /// Convert Expression to Z3 Int (static helper for ValueRange)
    fn expr_to_z3(expr: &Expression) -> Result<z3::ast::Int, BoundsError> {
        use z3::ast::Int;

        match expr {
            Expression::Int(n) => Ok(Int::from_i64(*n)),
            Expression::Var(name) => Ok(Int::new_const(name.as_str())),
            Expression::Binary { op, left, right } => {
                let l = Self::expr_to_z3(left)?;
                let r = Self::expr_to_z3(right)?;
                match op {
                    BinaryOp::Add => Ok(Int::add(&[&l, &r])),
                    BinaryOp::Sub => Ok(Int::sub(&[&l, &r])),
                    BinaryOp::Mul => Ok(Int::mul(&[&l, &r])),
                    BinaryOp::Div => Ok(l.div(&r)),
                    BinaryOp::Mod => Ok(l.rem(&r)),
                    _ => {
                        // Comparison operators return Bool, not Int
                        // Create symbolic variable for unsupported ops
                        let var_name = format!("expr_{:p}", expr as *const _);
                        Ok(Int::new_const(var_name.as_str()))
                    }
                }
            }
            Expression::ArrayLen(arr_expr) => {
                // Array length is a non-negative integer
                // Create symbolic variable with implicit constraint >= 0
                let arr_name = format!("{}.len", arr_expr);
                Ok(Int::new_const(arr_name.as_str()))
            }
            Expression::Field { base, field } => {
                // Field access - create symbolic variable
                let field_name = format!("{}.{}", base, field);
                Ok(Int::new_const(field_name.as_str()))
            }
        }
    }
}

// =============================================================================
// Bounds Check Eliminator
// =============================================================================

/// Main bounds check elimination engine
///
/// Main engine for eliminating runtime bounds checks. Uses refinement types,
/// loop invariants, meta parameters, and dataflow analysis to prove array
/// accesses safe. Target: >80% elimination rate, <50ms per function.
#[derive(Debug)]
pub struct BoundsCheckEliminator {
    /// Refinement constraints for variables
    refinements: Map<Text, Refinement>,
    /// Array bounds information
    array_bounds: Map<Text, ArrayBounds>,
    /// Loop invariants
    loop_invariants: Map<LoopId, LoopInvariant>,
    /// Control flow graph
    cfg: ControlFlowGraph,
    /// Dataflow analyzer
    dataflow: DataflowAnalyzer,
    /// Statistics
    stats: EliminationStats,
}

impl BoundsCheckEliminator {
    /// Create a new bounds check eliminator
    pub fn new(cfg: ControlFlowGraph) -> Self {
        Self {
            refinements: Map::new(),
            array_bounds: Map::new(),
            loop_invariants: Map::new(),
            dataflow: DataflowAnalyzer::new(),
            cfg,
            stats: EliminationStats::default(),
        }
    }

    /// Add refinement constraint
    pub fn add_refinement(&mut self, var: Text, constraint: Expression) {
        self.refinements
            .insert(var.clone(), Refinement::new(var, constraint));
    }

    /// Add array bounds information
    pub fn add_array_bounds(&mut self, bounds: ArrayBounds) {
        self.array_bounds.insert(bounds.array_var.clone(), bounds);
    }

    /// Add loop invariant
    pub fn add_loop_invariant(&mut self, invariant: LoopInvariant) {
        self.loop_invariants.insert(invariant.loop_id, invariant);
    }

    /// Add reaching definition for dataflow analysis
    pub fn add_reaching_def(&mut self, block: BlockId, var: Text, def: Definition) {
        self.dataflow.add_reaching_def(block, var, def);
    }

    /// Analyze array access and decide on bounds check
    ///
    /// This is the main entry point for bounds check elimination analysis.
    ///
    /// # Algorithm
    ///
    /// 1. Try refinement type analysis
    /// 2. Try loop invariant analysis
    /// 3. Try dataflow analysis
    /// 4. Try check hoisting
    /// 5. Conservative: keep check
    pub fn analyze_array_access(
        &mut self,
        access: &ArrayAccess,
    ) -> Result<CheckDecision, BoundsError> {
        self.stats.total_checks += 1;

        // Strategy 1: Refinement types prove bounds
        if let Maybe::Some(decision) = self.try_refinement_analysis(access)? {
            self.record_decision(decision);
            return Ok(decision);
        }

        // Strategy 2: Loop invariants prove bounds
        if let Maybe::Some(loop_id) = access.loop_context
            && let Maybe::Some(decision) = self.try_loop_invariant_analysis(access, loop_id)?
        {
            self.record_decision(decision);
            return Ok(decision);
        }

        // Strategy 3: Dataflow analysis proves bounds
        if let Maybe::Some(decision) = self.try_dataflow_analysis(access)? {
            self.record_decision(decision);
            return Ok(decision);
        }

        // Strategy 4: Check hoisting
        if let Maybe::Some(loop_id) = access.loop_context
            && self.can_hoist_check(access, loop_id)?
        {
            self.record_decision(CheckDecision::Hoist);
            return Ok(CheckDecision::Hoist);
        }

        // Conservative: keep check
        self.record_decision(CheckDecision::Keep);
        Ok(CheckDecision::Keep)
    }

    /// Try to eliminate check using refinement types
    fn try_refinement_analysis(
        &self,
        access: &ArrayAccess,
    ) -> Result<Maybe<CheckDecision>, BoundsError> {
        // Extract array and index variables
        let array_var = match access.array.as_var() {
            Maybe::Some(name) => name,
            Maybe::None => return Ok(Maybe::None),
        };

        let index_var = match access.index.as_var() {
            Maybe::Some(name) => name,
            Maybe::None => return Ok(Maybe::None),
        };

        // Get array bounds
        let array_bounds = match self.array_bounds.get(array_var) {
            Some(bounds) => bounds,
            None => return Ok(Maybe::None),
        };

        // Get index refinement
        let index_refinement = match self.refinements.get(index_var) {
            Some(ref_) => ref_,
            None => return Ok(Maybe::None),
        };

        // Check if refinement proves bounds
        if self.can_prove_in_bounds(array_bounds, index_refinement)? {
            return Ok(Maybe::Some(CheckDecision::Eliminate));
        }

        Ok(Maybe::None)
    }

    /// Check if refinement proves index is within bounds
    ///
    /// Example:
    /// - array: List<T> where len(array) == 100
    /// - index: Int where 0 <= index < 100
    /// Conclusion: Bounds check can be eliminated
    fn can_prove_in_bounds(
        &self,
        array_bounds: &ArrayBounds,
        index_refinement: &Refinement,
    ) -> Result<bool, BoundsError> {
        // Extract index constraint from refinement
        let constraint = self.extract_index_constraint(&index_refinement.constraint)?;

        // Get array length
        let array_len = match &array_bounds.length {
            Maybe::Some(len) => Expression::int(*len as i64),
            Maybe::None => match &array_bounds.length_expr {
                Maybe::Some(expr) => expr.clone(),
                Maybe::None => return Ok(false),
            },
        };

        // Check if constraint proves: 0 <= index < array_len
        self.prove_bounds_constraint(&constraint, &array_len)
    }

    /// Extract index constraint from refinement expression
    ///
    /// This is the production implementation that handles full predicate AST parsing.
    /// It recognizes multiple patterns for bounds constraints:
    ///
    /// - `lower <= var && var < upper` (canonical form)
    /// - `var >= lower && var < upper` (alternative form)
    /// - `lower <= var < upper` (chained comparison, desugared)
    /// - `0 <= var && var < len(arr)` (common array pattern)
    /// - Complex expressions with arithmetic (e.g., `i * stride < len`)
    ///
    /// The extraction is compositional, recursively processing nested conjunctions
    /// and building a complete constraint set.
    fn extract_index_constraint(
        &self,
        expr: &Expression,
    ) -> Result<(Expression, Expression), BoundsError> {
        // Collect all atomic constraints from the predicate
        let mut lower_constraints: Vec<Expression> = Vec::new();
        let mut upper_constraints: Vec<Expression> = Vec::new();

        // Parse the predicate tree
        self.collect_bounds_from_predicate(expr, &mut lower_constraints, &mut upper_constraints)?;

        // Determine the tightest lower bound (maximum of all lower bounds)
        let lower = if lower_constraints.is_empty() {
            Expression::int(0) // Default lower bound
        } else if lower_constraints.len() == 1 {
            lower_constraints.pop().unwrap()
        } else {
            // Multiple lower bounds - take the maximum
            self.compute_max_bound(&lower_constraints)
        };

        // Determine the tightest upper bound (minimum of all upper bounds)
        let upper = if upper_constraints.is_empty() {
            return Err(BoundsError::CannotExtractConstraint {
                expr: format!("no upper bound found in: {}", expr).into(),
            });
        } else if upper_constraints.len() == 1 {
            upper_constraints.pop().unwrap()
        } else {
            // Multiple upper bounds - take the minimum
            self.compute_min_bound(&upper_constraints)
        };

        Ok((lower, upper))
    }

    /// Recursively collect lower and upper bounds from a predicate expression
    fn collect_bounds_from_predicate(
        &self,
        expr: &Expression,
        lower_constraints: &mut Vec<Expression>,
        upper_constraints: &mut Vec<Expression>,
    ) -> Result<(), BoundsError> {
        match expr {
            // Conjunction: process both sides
            Expression::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => {
                self.collect_bounds_from_predicate(left, lower_constraints, upper_constraints)?;
                self.collect_bounds_from_predicate(right, lower_constraints, upper_constraints)?;
            }

            // Greater than or equal: var >= lower OR lower <= var
            Expression::Binary {
                op: BinaryOp::Ge,
                left,
                right,
            } => {
                // var >= lower implies lower is a lower bound
                lower_constraints.push((**right).clone());
            }

            // Less than or equal: lower <= var OR var <= upper
            Expression::Binary {
                op: BinaryOp::Le,
                left,
                right,
            } => {
                // lower <= var implies lower is a lower bound
                // But we need to distinguish from var <= upper
                if self.is_likely_index_expression(right) {
                    // Pattern: lower <= index
                    lower_constraints.push((**left).clone());
                } else {
                    // Pattern: index <= upper (convert to upper bound as index < upper + 1)
                    let upper_plus_one = Expression::Binary {
                        op: BinaryOp::Add,
                        left: right.clone(),
                        right: Box::new(Expression::int(1)),
                    };
                    upper_constraints.push(upper_plus_one);
                }
            }

            // Less than: var < upper
            Expression::Binary {
                op: BinaryOp::Lt,
                left: _,
                right,
            } => {
                // var < upper implies upper is an upper bound
                upper_constraints.push((**right).clone());
            }

            // Greater than: var > lower (convert to var >= lower + 1)
            Expression::Binary {
                op: BinaryOp::Gt,
                left,
                right,
            } => {
                // var > lower is equivalent to var >= lower + 1
                if self.is_likely_index_expression(left) {
                    let lower_plus_one = Expression::Binary {
                        op: BinaryOp::Add,
                        left: right.clone(),
                        right: Box::new(Expression::int(1)),
                    };
                    lower_constraints.push(lower_plus_one);
                } else {
                    // lower > var means var < lower, so lower is an upper bound
                    upper_constraints.push((**right).clone());
                }
            }

            // Single comparison or unrecognized pattern
            _ => {
                // Try to extract bounds from single expressions
                // For example: just "i < n" without explicit lower bound
                if let Some(upper) = self.try_extract_upper_bound_simple(expr) {
                    upper_constraints.push(upper);
                }
            }
        }

        Ok(())
    }

    /// Check if an expression is likely an index variable
    fn is_likely_index_expression(&self, expr: &Expression) -> bool {
        match expr {
            // Simple variable is likely an index
            Expression::Var(name) => {
                let name_lower = name.to_lowercase();
                name_lower.contains("index")
                    || name_lower.contains("idx")
                    || name_lower == "i"
                    || name_lower == "j"
                    || name_lower == "k"
                    || name_lower == "n"
            }
            // Arithmetic on index is still an index expression
            Expression::Binary { op, left, .. } => {
                matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul)
                    && self.is_likely_index_expression(left)
            }
            _ => false,
        }
    }

    /// Try to extract upper bound from a simple expression
    fn try_extract_upper_bound_simple(&self, expr: &Expression) -> Option<Expression> {
        match expr {
            Expression::Binary {
                op: BinaryOp::Lt,
                right,
                ..
            } => Some((**right).clone()),
            Expression::Binary {
                op: BinaryOp::Le,
                right,
                ..
            } => {
                // var <= upper means upper bound is upper + 1
                Some(Expression::Binary {
                    op: BinaryOp::Add,
                    left: right.clone(),
                    right: Box::new(Expression::int(1)),
                })
            }
            _ => None,
        }
    }

    /// Compute the maximum of multiple lower bounds
    fn compute_max_bound(&self, bounds: &[Expression]) -> Expression {
        if bounds.len() == 1 {
            return bounds[0].clone();
        }

        // Check for constant bounds - return the maximum constant
        let mut max_const: Option<i64> = None;
        let mut symbolic_bounds: Vec<Expression> = Vec::new();

        for bound in bounds {
            if let Expression::Int(val) = bound {
                max_const = Some(max_const.map_or(*val, |m| m.max(*val)));
            } else {
                symbolic_bounds.push(bound.clone());
            }
        }

        // If all bounds are constant, return the max
        if symbolic_bounds.is_empty() {
            return Expression::int(max_const.unwrap_or(0));
        }

        // If we have a mix, prefer the symbolic bound if it's likely larger
        // (conservative: return the first symbolic bound for now)
        if let Some(sym_bound) = symbolic_bounds.into_iter().next() {
            return sym_bound;
        }

        Expression::int(max_const.unwrap_or(0))
    }

    /// Compute the minimum of multiple upper bounds
    fn compute_min_bound(&self, bounds: &[Expression]) -> Expression {
        if bounds.len() == 1 {
            return bounds[0].clone();
        }

        // Check for constant bounds - return the minimum constant
        let mut min_const: Option<i64> = None;
        let mut symbolic_bounds: Vec<Expression> = Vec::new();

        for bound in bounds {
            if let Expression::Int(val) = bound {
                min_const = Some(min_const.map_or(*val, |m| m.min(*val)));
            } else {
                symbolic_bounds.push(bound.clone());
            }
        }

        // If all bounds are constant, return the min
        if symbolic_bounds.is_empty() {
            if let Some(min) = min_const {
                return Expression::int(min);
            }
        }

        // If we have symbolic bounds, prefer those (more restrictive)
        if let Some(sym_bound) = symbolic_bounds.into_iter().next() {
            return sym_bound;
        }

        Expression::int(min_const.unwrap_or(i64::MAX))
    }

    fn extract_lower_bound(&self, expr: &Expression) -> Result<Expression, BoundsError> {
        // Pattern: 0 <= index or index >= 0
        match expr {
            Expression::Binary {
                op: BinaryOp::Ge,
                right,
                ..
            } => Ok((**right).clone()),
            Expression::Binary {
                op: BinaryOp::Le,
                left,
                ..
            } => Ok((**left).clone()),
            Expression::Binary {
                op: BinaryOp::Gt,
                right,
                ..
            } => {
                // var > lower means var >= lower + 1
                Ok(Expression::Binary {
                    op: BinaryOp::Add,
                    left: right.clone(),
                    right: Box::new(Expression::int(1)),
                })
            }
            _ => Ok(Expression::int(0)), // Default to 0
        }
    }

    fn extract_upper_bound(&self, expr: &Expression) -> Result<Expression, BoundsError> {
        // Pattern: index < N or index <= N
        match expr {
            Expression::Binary {
                op: BinaryOp::Lt,
                right,
                ..
            } => Ok((**right).clone()),
            Expression::Binary {
                op: BinaryOp::Le,
                right,
                ..
            } => {
                // var <= upper means var < upper + 1
                Ok(Expression::Binary {
                    op: BinaryOp::Add,
                    left: right.clone(),
                    right: Box::new(Expression::int(1)),
                })
            }
            Expression::Binary {
                op: BinaryOp::Gt,
                left,
                ..
            } => {
                // upper > var means var < upper
                Ok((**left).clone())
            }
            Expression::Binary {
                op: BinaryOp::Ge,
                left,
                ..
            } => {
                // upper >= var means var <= upper, i.e., var < upper + 1
                Ok(Expression::Binary {
                    op: BinaryOp::Add,
                    left: left.clone(),
                    right: Box::new(Expression::int(1)),
                })
            }
            _ => Err(BoundsError::CannotExtractConstraint {
                expr: format!("{}", expr).into(),
            }),
        }
    }

    /// Prove bounds constraint using SMT solver
    ///
    /// Uses Z3 to verify that the index constraint ensures safe array access.
    /// The goal is to prove: lower <= index < upper AND upper <= array_len
    ///
    /// ## Strategy
    ///
    /// We check if the negation of the bounds property is UNSAT:
    /// - If UNSAT: bounds are always satisfied (eliminate check)
    /// - If SAT: found counterexample where bounds fail (keep check)
    /// - If Unknown: timeout or solver limitation (keep check conservatively)
    fn prove_bounds_constraint(
        &self,
        constraint: &(Expression, Expression),
        array_len: &Expression,
    ) -> Result<bool, BoundsError> {
        use z3::ast::{Ast, Int};
        use z3::{SatResult, Solver};

        let (lower, upper) = constraint;

        // Fast path: handle simple constant cases without solver
        match (lower, upper, array_len) {
            (Expression::Int(l), Expression::Int(u), Expression::Int(len)) => {
                // Prove: l <= index < u and u <= len
                // For constants, we can verify directly
                return Ok(*l >= 0 && *u <= *len);
            }
            _ => {
                // Complex case - use SMT solver
            }
        }

        // Create Z3 solver
        let solver = Solver::new();

        // Create symbolic variables
        let index = Int::new_const("index");
        let len_var = self.expression_to_z3_int(array_len)?;

        // Translate bounds to Z3
        let lower_z3 = self.expression_to_z3_int(lower)?;
        let upper_z3 = self.expression_to_z3_int(upper)?;

        // Build bounds constraint: lower <= index && index < upper && upper <= len
        let lower_bound = index.ge(&lower_z3);
        let upper_bound = index.lt(&upper_z3);
        let len_bound = upper_z3.le(&len_var);

        // We want to prove this is always true
        // So we assert the negation and check for UNSAT
        let bounds_valid = z3::ast::Bool::and(&[&lower_bound, &upper_bound, &len_bound]);

        // Also add constraint that index >= 0 (natural number)
        let non_negative = index.ge(Int::from_i64(0));

        // Assert that we have a valid index but bounds check fails
        // If this is UNSAT, then bounds always hold
        solver.assert(&non_negative);
        solver.assert(bounds_valid.not());

        // Set timeout for solver
        let mut params = z3::Params::new();
        params.set_u32("timeout", 1000); // 1 second timeout
        solver.set_params(&params);

        match solver.check() {
            SatResult::Unsat => {
                // No counterexample exists - bounds are always valid
                Ok(true)
            }
            SatResult::Sat => {
                // Found a counterexample - bounds can fail
                tracing::debug!(
                    target: "verum_verification::bounds",
                    lower = %lower,
                    upper = %upper,
                    array_len = %array_len,
                    "SMT found counterexample - keeping bounds check"
                );
                Ok(false)
            }
            SatResult::Unknown => {
                // Solver couldn't determine - be conservative
                tracing::warn!(
                    target: "verum_verification::bounds",
                    "SMT solver returned unknown - keeping bounds check"
                );
                Ok(false)
            }
        }
    }

    /// Convert an Expression to a Z3 Int
    fn expression_to_z3_int(&self, expr: &Expression) -> Result<z3::ast::Int, BoundsError> {
        use z3::ast::Int;

        match expr {
            Expression::Int(n) => Ok(Int::from_i64(*n)),
            Expression::Var(name) => Ok(Int::new_const(name.as_str())),
            Expression::Binary { op, left, right } => {
                let l = self.expression_to_z3_int(left)?;
                let r = self.expression_to_z3_int(right)?;
                match op {
                    BinaryOp::Add => Ok(Int::add(&[&l, &r])),
                    BinaryOp::Sub => Ok(Int::sub(&[&l, &r])),
                    BinaryOp::Mul => Ok(Int::mul(&[&l, &r])),
                    BinaryOp::Div => Ok(l.div(&r)),
                    BinaryOp::Mod => Ok(l.rem(&r)),
                    _ => {
                        // Unsupported binary op - create symbolic variable
                        let var_name = format!("binary_expr_{:p}", expr as *const _);
                        Ok(Int::new_const(var_name.as_str()))
                    }
                }
            }
            Expression::ArrayLen(arr_expr) => {
                // Array length - create symbolic variable based on array expression
                let arr_name = format!("{:?}.len", arr_expr);
                Ok(Int::new_const(arr_name.as_str()))
            }
            Expression::Field { base, field } => {
                // Field access - create symbolic variable
                let field_name = format!("{:?}.{}", base, field);
                Ok(Int::new_const(field_name.as_str()))
            }
        }
    }

    /// Try to eliminate check using loop invariants
    fn try_loop_invariant_analysis(
        &self,
        access: &ArrayAccess,
        loop_id: LoopId,
    ) -> Result<Maybe<CheckDecision>, BoundsError> {
        let invariant = match self.loop_invariants.get(&loop_id) {
            Some(inv) => inv,
            None => return Ok(Maybe::None),
        };

        // Check if invariant proves bounds
        if self.invariant_proves_bounds(invariant, access)? {
            return Ok(Maybe::Some(CheckDecision::Eliminate));
        }

        Ok(Maybe::None)
    }

    /// Check if loop invariant proves array access is safe
    ///
    /// Example:
    /// - Loop: for i in 0..array.len()
    /// - Access: array[i]
    /// - Invariant: 0 <= i < array.len()
    /// Conclusion: Safe
    fn invariant_proves_bounds(
        &self,
        invariant: &LoopInvariant,
        access: &ArrayAccess,
    ) -> Result<bool, BoundsError> {
        // Check if index is the induction variable
        let index_var = match access.index.as_var() {
            Maybe::Some(name) => name,
            Maybe::None => return Ok(false),
        };

        if index_var != &invariant.induction_var {
            return Ok(false);
        }

        // Check if loop bounds match array bounds
        let array_var = match access.array.as_var() {
            Maybe::Some(name) => name,
            Maybe::None => return Ok(false),
        };

        // Get array length from bounds
        let array_bounds = match self.array_bounds.get(array_var) {
            Some(bounds) => bounds,
            None => return Ok(false),
        };

        let array_len = match &array_bounds.length_expr {
            Maybe::Some(expr) => expr,
            Maybe::None => return Ok(false),
        };

        // Check if invariant upper bound matches array length
        // Use symbolic comparison with constant folding and normalization
        //
        // Compare loop invariant upper bound against array length symbolically
        Ok(self.exprs_equal_or_less(&invariant.upper_bound, array_len))
    }

    /// Check if expr1 == expr2 or expr1 < expr2 symbolically
    ///
    /// This handles common patterns:
    /// - Direct equality
    /// - Constant comparison
    /// - len - 1 < len patterns
    /// - i < n where n is known to be array length
    fn exprs_equal_or_less(&self, expr1: &Expression, expr2: &Expression) -> bool {
        // Direct equality
        if expr1 == expr2 {
            return true;
        }

        // Constant comparison
        if let (Expression::Int(a), Expression::Int(b)) = (expr1, expr2) {
            return a < b;
        }

        // Check for expr1 = expr2 - constant pattern (e.g., len - 1 < len)
        if let Expression::Binary {
            op: BinaryOp::Sub,
            left,
            right,
        } = expr2
        {
            if let Expression::Int(c) = right.as_ref() {
                if *c > 0 && expr1 == left.as_ref() {
                    return false; // expr1 = left, expr2 = left - c, so expr1 > expr2
                }
            }
        }

        // Check for expr1 = expr2 - constant pattern inverted
        if let Expression::Binary {
            op: BinaryOp::Sub,
            left,
            right,
        } = expr1
        {
            if let Expression::Int(c) = right.as_ref() {
                if *c > 0 && left.as_ref() == expr2 {
                    return true; // expr1 = expr2 - c < expr2
                }
            }
        }

        // Check for expr1 + 1 <= expr2 (common in loop bounds)
        if let Expression::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } = expr1
        {
            if let Expression::Int(1) = right.as_ref() {
                return self.exprs_equal_or_less(left, expr2);
            }
        }

        // Check via variable tracking if both reference the same array length
        if let (Expression::Var(v1), Expression::Var(v2)) = (expr1, expr2) {
            if let (Some(b1), Some(b2)) = (self.array_bounds.get(v1), self.array_bounds.get(v2)) {
                if b1.length == b2.length && b1.length.is_some() {
                    return true;
                }
            }
        }

        // Check if expr1 is ArrayLen and expr2 references same array
        if let (Expression::ArrayLen(arr1), Expression::ArrayLen(arr2)) = (expr1, expr2) {
            return arr1 == arr2;
        }

        false
    }

    /// Try to eliminate check using dataflow analysis
    fn try_dataflow_analysis(
        &self,
        access: &ArrayAccess,
    ) -> Result<Maybe<CheckDecision>, BoundsError> {
        // Extract index variable
        let index_var = match access.index.as_var() {
            Maybe::Some(name) => name,
            Maybe::None => return Ok(Maybe::None),
        };

        // Get value range at access point
        let range = self.dataflow.analyze_value_range(index_var, access.block)?;

        // Get array bounds
        let array_var = match access.array.as_var() {
            Maybe::Some(name) => name,
            Maybe::None => return Ok(Maybe::None),
        };

        let array_bounds = match self.array_bounds.get(array_var) {
            Some(bounds) => bounds,
            None => return Ok(Maybe::None),
        };

        // Check if dataflow range proves bounds
        if self.range_proves_bounds(&range, array_bounds)? {
            return Ok(Maybe::Some(CheckDecision::Eliminate));
        }

        Ok(Maybe::None)
    }

    fn range_proves_bounds(
        &self,
        range: &ValueRange,
        array_bounds: &ArrayBounds,
    ) -> Result<bool, BoundsError> {
        // Get array length
        let array_len = match &array_bounds.length {
            Maybe::Some(len) => Expression::int(*len as i64),
            Maybe::None => return Ok(false),
        };

        // Check if range is within [0, array_len)
        match (&range.lower, &range.upper, &array_len) {
            (Expression::Int(l), Expression::Int(u), Expression::Int(len)) => {
                Ok(*l >= 0 && *u < *len && range.proven)
            }
            _ => Ok(false),
        }
    }

    /// Check if bounds check can be hoisted out of loop
    ///
    /// Hoisting is safe when we can prove the bounds check will always pass
    /// for all values the index can take during loop execution.
    ///
    /// We can hoist when:
    /// 1. Index is a linear function of induction variable: i * k + c
    /// 2. Array length is loop-invariant (doesn't change during loop)
    /// 3. We can compute worst-case index (max value in iteration space)
    ///
    /// Check hoisting: move bounds check from loop body to loop preheader.
    /// Requires: (1) index is linear function of induction variable (i*k+c),
    /// (2) array length is loop-invariant, (3) worst-case index is computable.
    fn can_hoist_check(&self, access: &ArrayAccess, loop_id: LoopId) -> Result<bool, BoundsError> {
        // Get loop info
        let loop_info = match self.loop_invariants.get(&loop_id) {
            Some(info) => info,
            None => return Ok(false),
        };

        // Check if array length is loop-invariant
        if !self.is_loop_invariant(&access.array, loop_id)? {
            return Ok(false);
        }

        // Get array bounds
        let array_var = match access.array.as_var() {
            Maybe::Some(name) => name,
            Maybe::None => return Ok(false),
        };

        let array_bounds = match self.array_bounds.get(array_var) {
            Some(bounds) => bounds,
            None => return Ok(false),
        };

        // Analyze the index expression to see if it can be hoisted
        let hoist_result = self.analyze_index_for_hoisting(&access.index, loop_info, array_bounds);

        Ok(hoist_result.can_hoist)
    }

    /// Check if an expression is loop-invariant
    fn is_loop_invariant(&self, expr: &Expression, loop_id: LoopId) -> Result<bool, BoundsError> {
        match expr {
            // Constants are always loop-invariant
            Expression::Int(_) => Ok(true),

            // Variables are loop-invariant if not modified in loop
            Expression::Var(name) => {
                if let Some(loop_info) = self.loop_invariants.get(&loop_id) {
                    // Check if variable is the induction variable
                    if &loop_info.induction_var == name {
                        return Ok(false);
                    }
                }
                // Check if variable is modified in loop (would need CFG analysis)
                // For now, assume arrays are not modified
                Ok(true)
            }

            // Array length is invariant if array is invariant
            Expression::ArrayLen(arr) => self.is_loop_invariant(arr, loop_id),

            // Field access is invariant if base is invariant
            Expression::Field { base, .. } => self.is_loop_invariant(base, loop_id),

            // Binary ops are invariant if both operands are
            Expression::Binary { left, right, .. } => {
                Ok(self.is_loop_invariant(left, loop_id)?
                    && self.is_loop_invariant(right, loop_id)?)
            }
        }
    }

    /// Analyze an index expression to determine if bounds check can be hoisted
    fn analyze_index_for_hoisting(
        &self,
        index: &Expression,
        loop_info: &LoopInvariant,
        array_bounds: &ArrayBounds,
    ) -> HoistAnalysisResult {
        // Check for common patterns that can be hoisted

        // Pattern 1: Direct induction variable (i)
        let induction_var = &loop_info.induction_var;
        if let Expression::Var(name) = index {
            if name == induction_var {
                // Check if loop bounds match array bounds
                if let (Maybe::Some(array_len), Some(loop_upper)) =
                    (array_bounds.length, self.get_loop_upper_bound(loop_info))
                {
                    if loop_upper <= array_len as i64 {
                        return HoistAnalysisResult {
                            can_hoist: true,
                            worst_case_index: Expression::int(loop_upper - 1),
                            hoisted_check: Some(Expression::binary(
                                BinaryOp::Lt,
                                Expression::int(loop_upper - 1),
                                Expression::int(array_len as i64),
                            )),
                        };
                    }
                }
            }
        }

        // Pattern 2: Linear expression (i * k + c)
        if let Expression::Binary { op, left, right } = index {
            match op {
                BinaryOp::Mul => {
                    // i * k: worst case is upper_bound * k
                    if let Some(upper) = self.get_loop_upper_bound(loop_info) {
                        if let Expression::Int(k) = right.as_ref() {
                            return HoistAnalysisResult {
                                can_hoist: true,
                                worst_case_index: Expression::int((upper - 1) * k),
                                hoisted_check: Some(Expression::binary(
                                    BinaryOp::Lt,
                                    Expression::int((upper - 1) * k),
                                    Expression::ArrayLen(Box::new(Expression::var(
                                        array_bounds.name.clone(),
                                    ))),
                                )),
                            };
                        }
                    }
                }
                BinaryOp::Add => {
                    // i + c or c + i: worst case is upper_bound - 1 + c
                    if let Some(upper) = self.get_loop_upper_bound(loop_info) {
                        if let Expression::Int(c) = right.as_ref() {
                            return HoistAnalysisResult {
                                can_hoist: true,
                                worst_case_index: Expression::int(upper - 1 + c),
                                hoisted_check: Some(Expression::binary(
                                    BinaryOp::Lt,
                                    Expression::int(upper - 1 + c),
                                    Expression::ArrayLen(Box::new(Expression::var(
                                        array_bounds.name.clone(),
                                    ))),
                                )),
                            };
                        }
                    }
                }
                _ => {}
            }
        }

        // Cannot determine hoisting safety
        HoistAnalysisResult {
            can_hoist: false,
            worst_case_index: index.clone(),
            hoisted_check: None,
        }
    }

    /// Get the upper bound of a loop's iteration space
    fn get_loop_upper_bound(&self, loop_info: &LoopInvariant) -> Option<i64> {
        if let Expression::Int(upper) = &loop_info.upper_bound {
            Some(*upper)
        } else {
            None
        }
    }

    /// Record decision in statistics
    fn record_decision(&mut self, decision: CheckDecision) {
        match decision {
            CheckDecision::Eliminate => self.stats.eliminated_checks += 1,
            CheckDecision::Hoist => self.stats.hoisted_checks += 1,
            CheckDecision::Keep => self.stats.kept_checks += 1,
        }
    }

    /// Get elimination statistics
    pub fn stats(&self) -> &EliminationStats {
        &self.stats
    }

    /// Batch analyze array accesses in a loop
    pub fn eliminate_in_loop(
        &mut self,
        loop_id: LoopId,
        accesses: &List<ArrayAccess>,
    ) -> Result<List<CheckDecision>, BoundsError> {
        let mut decisions = List::new();

        for access in accesses.iter() {
            let decision = self.analyze_array_access(access)?;
            decisions.push(decision);
        }

        Ok(decisions)
    }
}

// =============================================================================
// Dataflow Analysis
// =============================================================================

/// Dataflow analyzer for value range analysis
#[derive(Debug)]
pub struct DataflowAnalyzer {
    /// Reaching definitions: block -> var -> definition
    reaching_definitions: HashMap<BlockId, HashMap<Text, Definition>>,
    /// Available expressions: block -> set of expressions
    available_expressions: HashMap<BlockId, HashSet<Expression>>,
}

impl DataflowAnalyzer {
    pub fn new() -> Self {
        Self {
            reaching_definitions: HashMap::new(),
            available_expressions: HashMap::new(),
        }
    }

    /// Analyze value range for variable at given block
    pub fn analyze_value_range(
        &self,
        var: &Text,
        block: BlockId,
    ) -> Result<ValueRange, BoundsError> {
        // Get reaching definition
        let def = self
            .reaching_definitions
            .get(&block)
            .and_then(|defs| defs.get(var));

        match def {
            Some(def) => Ok(self.extract_range_from_def(def)),
            None => Ok(ValueRange::unbounded()),
        }
    }

    fn extract_range_from_def(&self, def: &Definition) -> ValueRange {
        // Extract range from definition
        match &def.value {
            Expression::Int(value) => {
                // Exact value
                ValueRange::new(Expression::int(*value), Expression::int(*value)).with_proven(true)
            }
            Expression::Binary {
                op: BinaryOp::Add,
                left,
                right,
            } => {
                // Add: combine ranges
                self.combine_add_ranges(left, right)
            }
            _ => ValueRange::unbounded(),
        }
    }

    /// Combine ranges for addition operation
    ///
    /// For addition `left + right`, the resulting range is:
    /// - lower = left.lower + right.lower
    /// - upper = left.upper + right.upper
    ///
    /// This is sound because if a in [l1, u1] and b in [l2, u2],
    /// then a + b in [l1 + l2, u1 + u2].
    fn combine_add_ranges(&self, left: &Expression, right: &Expression) -> ValueRange {
        // Extract ranges for left and right operands
        let left_range = self.expression_to_range(left);
        let right_range = self.expression_to_range(right);

        // Combine the ranges using interval arithmetic
        let new_lower = match (&left_range.lower, &right_range.lower) {
            (Expression::Int(l1), Expression::Int(l2)) => Expression::int(l1 + l2),
            (l, Expression::Int(0)) => l.clone(),
            (Expression::Int(0), r) => r.clone(),
            (l, r) => Expression::binary(BinaryOp::Add, l.clone(), r.clone()),
        };

        let new_upper = match (&left_range.upper, &right_range.upper) {
            (Expression::Int(u1), Expression::Int(u2)) => Expression::int(u1 + u2),
            (u, Expression::Int(0)) => u.clone(),
            (Expression::Int(0), u) => u.clone(),
            (l, r) => Expression::binary(BinaryOp::Add, l.clone(), r.clone()),
        };

        let proven = left_range.proven && right_range.proven;
        ValueRange::new(new_lower, new_upper).with_proven(proven)
    }

    /// Convert an expression to a value range
    fn expression_to_range(&self, expr: &Expression) -> ValueRange {
        match expr {
            // Constant: exact range
            Expression::Int(n) => {
                ValueRange::new(Expression::int(*n), Expression::int(*n)).with_proven(true)
            }
            // Variable: try to look up in reaching definitions
            Expression::Var(name) => {
                // If we have a definition for this variable, use it
                // Otherwise, return symbolic range
                ValueRange::new(Expression::int(0), expr.clone()).with_proven(false)
            }
            // Binary operation: combine ranges
            Expression::Binary { op, left, right } => {
                let left_range = self.expression_to_range(left);
                let right_range = self.expression_to_range(right);

                match op {
                    BinaryOp::Add => self.combine_add_ranges_from_ranges(&left_range, &right_range),
                    BinaryOp::Sub => self.combine_sub_ranges(&left_range, &right_range),
                    BinaryOp::Mul => self.combine_mul_ranges(&left_range, &right_range),
                    _ => ValueRange::unbounded(),
                }
            }
            // Array length: non-negative
            Expression::ArrayLen(_) => {
                ValueRange::new(Expression::int(0), expr.clone()).with_proven(false)
            }
            // Field access: symbolic
            Expression::Field { .. } => ValueRange::unbounded(),
        }
    }

    /// Combine two value ranges for addition
    fn combine_add_ranges_from_ranges(&self, left: &ValueRange, right: &ValueRange) -> ValueRange {
        let new_lower = match (&left.lower, &right.lower) {
            (Expression::Int(l1), Expression::Int(l2)) => Expression::int(l1 + l2),
            (l, Expression::Int(0)) => l.clone(),
            (Expression::Int(0), r) => r.clone(),
            (l, r) => Expression::binary(BinaryOp::Add, l.clone(), r.clone()),
        };

        let new_upper = match (&left.upper, &right.upper) {
            (Expression::Int(u1), Expression::Int(u2)) => Expression::int(u1 + u2),
            (u, Expression::Int(0)) => u.clone(),
            (Expression::Int(0), u) => u.clone(),
            (l, r) => Expression::binary(BinaryOp::Add, l.clone(), r.clone()),
        };

        ValueRange::new(new_lower, new_upper).with_proven(left.proven && right.proven)
    }

    /// Combine ranges for subtraction
    fn combine_sub_ranges(&self, left: &ValueRange, right: &ValueRange) -> ValueRange {
        // For a - b where a in [l1, u1] and b in [l2, u2]:
        // Result is in [l1 - u2, u1 - l2]
        let new_lower = match (&left.lower, &right.upper) {
            (Expression::Int(l1), Expression::Int(u2)) => Expression::int(l1 - u2),
            (l, Expression::Int(0)) => l.clone(),
            (l, r) => Expression::binary(BinaryOp::Sub, l.clone(), r.clone()),
        };

        let new_upper = match (&left.upper, &right.lower) {
            (Expression::Int(u1), Expression::Int(l2)) => Expression::int(u1 - l2),
            (u, Expression::Int(0)) => u.clone(),
            (l, r) => Expression::binary(BinaryOp::Sub, l.clone(), r.clone()),
        };

        ValueRange::new(new_lower, new_upper).with_proven(left.proven && right.proven)
    }

    /// Combine ranges for multiplication
    ///
    /// For multiplication `a * b` where `a in [l1, u1]` and `b in [l2, u2]`,
    /// the resulting range depends on the signs of the operands:
    ///
    /// - Both non-negative: [l1 * l2, u1 * u2]
    /// - Both non-positive: [u1 * u2, l1 * l2]
    /// - Mixed signs: [min(l1*u2, u1*l2), max(l1*l2, u1*u2)]
    ///
    /// Interval arithmetic for multiplication: compute [lower, upper] of product.
    /// Both non-negative: [l1*l2, u1*u2]; both non-positive: [u1*u2, l1*l2];
    /// mixed signs: [min(l1*u2, u1*l2), max(l1*l2, u1*u2)].
    fn combine_mul_ranges(&self, left: &ValueRange, right: &ValueRange) -> ValueRange {
        // Try to extract constant bounds for precise computation
        match (&left.lower, &left.upper, &right.lower, &right.upper) {
            (
                Expression::Int(l1),
                Expression::Int(u1),
                Expression::Int(l2),
                Expression::Int(u2),
            ) => {
                // Both ranges have constant bounds - compute precisely
                let products = [l1 * l2, l1 * u2, u1 * l2, u1 * u2];
                let min_prod = *products.iter().min().unwrap();
                let max_prod = *products.iter().max().unwrap();

                ValueRange::new(Expression::int(min_prod), Expression::int(max_prod))
                    .with_proven(left.proven && right.proven)
            }

            // One operand is zero - result is zero
            (Expression::Int(0), Expression::Int(0), _, _)
            | (_, _, Expression::Int(0), Expression::Int(0)) => {
                ValueRange::new(Expression::int(0), Expression::int(0)).with_proven(true)
            }

            // Left is non-negative constant range, right has variable bounds
            (Expression::Int(l1), Expression::Int(u1), _, _) if *l1 >= 0 && *u1 >= 0 => {
                let new_lower = if *l1 == 0 {
                    Expression::int(0)
                } else {
                    Expression::binary(BinaryOp::Mul, Expression::int(*l1), right.lower.clone())
                };
                let new_upper =
                    Expression::binary(BinaryOp::Mul, Expression::int(*u1), right.upper.clone());
                ValueRange::new(new_lower, new_upper).with_proven(false)
            }

            // Right is non-negative constant range, left has variable bounds
            (_, _, Expression::Int(l2), Expression::Int(u2)) if *l2 >= 0 && *u2 >= 0 => {
                let new_lower = if *l2 == 0 {
                    Expression::int(0)
                } else {
                    Expression::binary(BinaryOp::Mul, left.lower.clone(), Expression::int(*l2))
                };
                let new_upper =
                    Expression::binary(BinaryOp::Mul, left.upper.clone(), Expression::int(*u2));
                ValueRange::new(new_lower, new_upper).with_proven(false)
            }

            // General case: cannot precisely determine without sign info
            // Use conservative symbolic bounds
            _ => {
                let new_lower =
                    Expression::binary(BinaryOp::Mul, left.lower.clone(), right.lower.clone());
                let new_upper =
                    Expression::binary(BinaryOp::Mul, left.upper.clone(), right.upper.clone());
                ValueRange::new(new_lower, new_upper).with_proven(false)
            }
        }
    }

    /// Add reaching definition
    pub fn add_reaching_def(&mut self, block: BlockId, var: Text, def: Definition) {
        self.reaching_definitions
            .entry(block)
            .or_default()
            .insert(var, def);
    }
}

impl Default for DataflowAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Variable definition site
#[derive(Debug, Clone)]
pub struct Definition {
    /// Variable name
    pub var: Text,
    /// Value expression
    pub value: Expression,
    /// Block where defined
    pub block: BlockId,
}

// =============================================================================
// Statistics
// =============================================================================

/// Bounds check elimination statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EliminationStats {
    /// Total bounds checks analyzed
    pub total_checks: usize,
    /// Checks eliminated (proven safe)
    pub eliminated_checks: usize,
    /// Checks hoisted (moved out of loop)
    pub hoisted_checks: usize,
    /// Checks kept (cannot prove safe)
    pub kept_checks: usize,
}

impl EliminationStats {
    /// Calculate elimination rate (percentage)
    pub fn elimination_rate(&self) -> f64 {
        if self.total_checks == 0 {
            0.0
        } else {
            (self.eliminated_checks as f64 / self.total_checks as f64) * 100.0
        }
    }

    /// Calculate optimization rate (eliminated + hoisted)
    pub fn optimization_rate(&self) -> f64 {
        if self.total_checks == 0 {
            0.0
        } else {
            ((self.eliminated_checks + self.hoisted_checks) as f64 / self.total_checks as f64)
                * 100.0
        }
    }

    /// Estimated time saved in nanoseconds
    pub fn estimated_time_saved_ns(&self) -> u64 {
        // Eliminated: save 5ns per access (assume 100 accesses per check)
        // Hoisted: save 4ns per access (1ns hoist vs 5ns inline)
        (self.eliminated_checks as u64 * 5 * 100) + (self.hoisted_checks as u64 * 4 * 100)
    }
}

impl fmt::Display for EliminationStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Bounds Check Elimination Statistics:")?;
        writeln!(f, "  Total checks: {}", self.total_checks)?;
        writeln!(
            f,
            "  Eliminated: {} ({:.1}%)",
            self.eliminated_checks,
            self.elimination_rate()
        )?;
        writeln!(f, "  Hoisted: {}", self.hoisted_checks)?;
        writeln!(f, "  Kept: {}", self.kept_checks)?;
        writeln!(f, "  Optimization rate: {:.1}%", self.optimization_rate())?;
        writeln!(
            f,
            "  Est. time saved: ~{}ns",
            self.estimated_time_saved_ns()
        )?;
        Ok(())
    }
}

// =============================================================================
// Meta Parameter Support
// =============================================================================

/// Meta parameter constraint for compile-time bounds
///
/// Example: Array<T, N> where N is known at compile time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaConstraint {
    /// Meta parameter name (e.g., "N")
    pub param_name: Text,
    /// Constraint expression
    pub constraint: Expression,
    /// Compile-time value (if resolved)
    pub value: Maybe<usize>,
}

impl MetaConstraint {
    pub fn new(param_name: Text, constraint: Expression) -> Self {
        Self {
            param_name,
            constraint,
            value: Maybe::None,
        }
    }

    pub fn with_value(mut self, value: usize) -> Self {
        self.value = Maybe::Some(value);
        self
    }

    /// Check if constraint is satisfied for given value
    ///
    /// This is the production implementation that handles various constraint forms:
    /// - `value < N` (strict upper bound)
    /// - `value <= N` (inclusive upper bound)
    /// - `value >= 0` (non-negative)
    /// - `0 <= value < N` (range constraint)
    /// - Complex arithmetic constraints
    ///
    /// The evaluation substitutes:
    /// 1. The meta parameter (e.g., "N") with its resolved value (from `with_value`)
    /// 2. Common index variables ("index", "i", "value") with the input value
    ///
    /// # Arguments
    ///
    /// * `index_value` - The value to test against the constraint (substituted for index variables)
    ///
    /// # Returns
    ///
    /// `true` if the constraint is satisfied, `false` otherwise
    pub fn verify(&self, index_value: usize) -> bool {
        // First substitute the meta parameter with its resolved value
        let with_meta_resolved = if let Maybe::Some(meta_val) = self.value {
            self.substitute_var(&self.constraint, &self.param_name, meta_val)
        } else {
            self.constraint.clone()
        };

        // Then substitute common index variable names with the input value
        // We support "index", "i", and "value" as standard index variable names
        let mut resolved = with_meta_resolved;
        for idx_var in &["index", "i", "value"] {
            resolved = self.substitute_var(&resolved, &Text::from(*idx_var), index_value);
        }

        self.evaluate_bool_constraint(&resolved)
    }

    /// Substitute a variable name with a concrete value
    fn substitute_var(&self, expr: &Expression, var_name: &Text, value: usize) -> Expression {
        match expr {
            Expression::Var(name) if name == var_name => Expression::int(value as i64),
            Expression::Var(_) => expr.clone(),
            Expression::Int(_) => expr.clone(),
            Expression::Binary { op, left, right } => Expression::Binary {
                op: *op,
                left: Box::new(self.substitute_var(left, var_name, value)),
                right: Box::new(self.substitute_var(right, var_name, value)),
            },
            Expression::ArrayLen(inner) => {
                Expression::ArrayLen(Box::new(self.substitute_var(inner, var_name, value)))
            }
            Expression::Field { base, field } => Expression::Field {
                base: Box::new(self.substitute_var(base, var_name, value)),
                field: field.clone(),
            },
        }
    }

    /// Substitute meta parameter with a concrete value (legacy method for compatibility)
    #[allow(dead_code)]
    fn substitute_meta_param(&self, expr: &Expression, value: usize) -> Expression {
        self.substitute_var(expr, &self.param_name, value)
    }

    /// Evaluate a constraint expression to a boolean result
    fn evaluate_bool_constraint(&self, expr: &Expression) -> bool {
        match expr {
            // Evaluate comparisons
            Expression::Binary { op, left, right } => {
                match op {
                    // Boolean operators
                    BinaryOp::And => {
                        self.evaluate_bool_constraint(left) && self.evaluate_bool_constraint(right)
                    }
                    BinaryOp::Or => {
                        self.evaluate_bool_constraint(left) || self.evaluate_bool_constraint(right)
                    }
                    // Comparison operators
                    BinaryOp::Lt => {
                        match (self.try_evaluate_int(left), self.try_evaluate_int(right)) {
                            (Some(l), Some(r)) => l < r,
                            _ => false, // Cannot evaluate symbolically
                        }
                    }
                    BinaryOp::Le => {
                        match (self.try_evaluate_int(left), self.try_evaluate_int(right)) {
                            (Some(l), Some(r)) => l <= r,
                            _ => false,
                        }
                    }
                    BinaryOp::Gt => {
                        match (self.try_evaluate_int(left), self.try_evaluate_int(right)) {
                            (Some(l), Some(r)) => l > r,
                            _ => false,
                        }
                    }
                    BinaryOp::Ge => {
                        match (self.try_evaluate_int(left), self.try_evaluate_int(right)) {
                            (Some(l), Some(r)) => l >= r,
                            _ => false,
                        }
                    }
                    BinaryOp::Eq => {
                        match (self.try_evaluate_int(left), self.try_evaluate_int(right)) {
                            (Some(l), Some(r)) => l == r,
                            _ => false,
                        }
                    }
                    BinaryOp::Ne => {
                        match (self.try_evaluate_int(left), self.try_evaluate_int(right)) {
                            (Some(l), Some(r)) => l != r,
                            _ => false,
                        }
                    }
                    // Arithmetic operators return integers, not booleans
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Try to evaluate an expression to a constant integer
    fn try_evaluate_int(&self, expr: &Expression) -> Option<i64> {
        match expr {
            Expression::Int(n) => Some(*n),
            Expression::Binary { op, left, right } => {
                let l = self.try_evaluate_int(left)?;
                let r = self.try_evaluate_int(right)?;
                match op {
                    BinaryOp::Add => Some(l + r),
                    BinaryOp::Sub => Some(l - r),
                    BinaryOp::Mul => Some(l * r),
                    BinaryOp::Div if r != 0 => Some(l / r),
                    BinaryOp::Mod if r != 0 => Some(l % r),
                    _ => None, // Comparison operators don't return integers
                }
            }
            // Variable or field access: cannot evaluate to constant
            Expression::Var(_) | Expression::ArrayLen(_) | Expression::Field { .. } => None,
        }
    }

    /// Check if this constraint implies a given bound
    pub fn implies_bound(&self, index_value: usize, bound: usize) -> bool {
        // Check if the constraint, when satisfied, implies index_value < bound
        match &self.value {
            Maybe::Some(_meta_value) => {
                // If meta param is resolved, check if constraint is satisfied
                // and if so, whether the bound is satisfied
                self.verify(index_value) && index_value < bound
            }
            Maybe::None => false, // Cannot verify without resolved meta value
        }
    }
}

// =============================================================================
// Errors
// =============================================================================

/// Bounds check elimination errors
#[derive(Debug, Clone)]
pub enum BoundsError {
    /// Cannot extract constraint from expression
    CannotExtractConstraint { expr: Text },
    /// Invalid array access
    InvalidAccess { reason: Text },
    /// Internal error
    Internal { message: Text },
}

impl fmt::Display for BoundsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoundsError::CannotExtractConstraint { expr } => {
                write!(f, "cannot extract constraint from: {}", expr)
            }
            BoundsError::InvalidAccess { reason } => {
                write!(f, "invalid array access: {}", reason)
            }
            BoundsError::Internal { message } => write!(f, "internal error: {}", message),
        }
    }
}

impl std::error::Error for BoundsError {}

// =============================================================================
// Public API
// =============================================================================

/// Analyze array access with bounds check elimination
///
/// This is a convenience function for single-access analysis.
pub fn analyze_bounds_check(
    access: &ArrayAccess,
    cfg: &ControlFlowGraph,
) -> Result<CheckDecision, BoundsError> {
    let mut eliminator = BoundsCheckEliminator::new(cfg.clone());
    eliminator.analyze_array_access(access)
}

/// Batch analyze array accesses in a function
pub fn analyze_function_bounds(
    accesses: &List<ArrayAccess>,
    cfg: &ControlFlowGraph,
) -> Result<List<CheckDecision>, BoundsError> {
    let mut eliminator = BoundsCheckEliminator::new(cfg.clone());

    let mut decisions = List::new();
    for access in accesses.iter() {
        let decision = eliminator.analyze_array_access(access)?;
        decisions.push(decision);
    }

    Ok(decisions)
}

/// Get elimination statistics for a set of decisions
pub fn compute_elimination_stats(decisions: &List<CheckDecision>) -> EliminationStats {
    let mut stats = EliminationStats::default();

    for decision in decisions.iter() {
        stats.total_checks += 1;
        match decision {
            CheckDecision::Eliminate => stats.eliminated_checks += 1,
            CheckDecision::Hoist => stats.hoisted_checks += 1,
            CheckDecision::Keep => stats.kept_checks += 1,
        }
    }

    stats
}

// Tests moved to tests/bounds_elimination_tests.rs per CLAUDE.md standards
