//! Value Tracking for Concrete Value Analysis
//!
//! Tracks concrete values (constants, ranges, symbolic expressions) through the CFG
//! to refine escape decisions. When a branch condition is known to be always-true or
//! always-false, infeasible paths are pruned, reducing false escapes.
//!
//! This module implements concrete value tracking through control flow graphs
//! to enable more precise escape analysis. By tracking constant propagation,
//! range analysis, and symbolic execution, we can refine escape decisions
//! based on actual runtime values.
//!
//! # Core Algorithm
//!
//! Value tracking operates in three phases:
//!
//! 1. **Value Extraction**: Extract concrete values from assignments and constants
//! 2. **Value Propagation**: Propagate values through CFG using dataflow analysis
//! 3. **Predicate Evaluation**: Evaluate path predicates with concrete values
//!
//! # Key Benefits
//!
//! - **Precise Range Analysis**: Prove array bounds never escape
//! - **Constant Folding**: Evaluate conditions at compile-time
//! - **Path Pruning**: Eliminate infeasible paths early
//! - **Symbolic Tracking**: Handle complex expressions
//!
//! # Performance Target
//!
//! - Typical function: < 200μs
//! - With Z3 integration: < 1ms
//! - Overhead vs basic analysis: < 10%
//!
//! # Example
//!
//! ```rust,ignore
//! fn conditional_escape(flag: bool, size: usize) {
//!     let data = vec![0; size];
//!
//!     if size < 100 {  // Value tracking: size ∈ [0, 99]
//!         process(&data);  // ✅ Small allocation, can prove no escape
//!     } else {
//!         store(&data);    // ❌ Large allocation, may escape
//!     }
//! }
//! ```

use std::fmt;
use verum_common::{List, Map, Maybe, Set};

use crate::analysis::BlockId;

// ==================================================================================
// Core Value Types
// ==================================================================================

/// Concrete value representation
///
/// Represents known constant values that can be tracked through the CFG.
/// Used for constant propagation and concrete value analysis.
///
/// Known constant values tracked through CFG for constant propagation.
/// Enables pruning of infeasible paths in escape analysis.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConcreteValue {
    /// Integer constant
    Integer(i64),

    /// Boolean constant
    Boolean(bool),

    /// Unsigned integer constant
    Unsigned(u64),

    /// Null/None value
    Null,

    /// Unknown/bottom value (no information)
    Unknown,

    /// Top value (multiple possible values - conservative)
    Top,
}

impl ConcreteValue {
    /// Check if value is known (not Unknown or Top)
    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, ConcreteValue::Unknown | ConcreteValue::Top)
    }

    /// Check if value is constant (single known value)
    #[must_use]
    pub fn is_constant(&self) -> bool {
        matches!(
            self,
            ConcreteValue::Integer(_)
                | ConcreteValue::Boolean(_)
                | ConcreteValue::Unsigned(_)
                | ConcreteValue::Null
        )
    }

    /// Merge two concrete values (lattice join)
    ///
    /// # Lattice Structure
    /// ```text
    ///        Top (all values)
    ///       /   \
    ///   const1  const2 ...
    ///       \   /
    ///      Unknown (no values)
    /// ```
    #[must_use]
    pub fn merge(&self, other: &ConcreteValue) -> ConcreteValue {
        match (self, other) {
            // Same value -> keep it
            (a, b) if a == b => a.clone(),

            // Unknown is bottom (absorbs nothing)
            (ConcreteValue::Unknown, x) | (x, ConcreteValue::Unknown) => x.clone(),

            // Top is top (absorbs everything)
            (ConcreteValue::Top, _) | (_, ConcreteValue::Top) => ConcreteValue::Top,

            // Different concrete values -> Top
            _ => ConcreteValue::Top,
        }
    }

    /// Evaluate binary operation
    pub fn eval_binop(&self, op: BinaryOp, other: &ConcreteValue) -> ConcreteValue {
        match (self, other) {
            (ConcreteValue::Integer(a), ConcreteValue::Integer(b)) => match op {
                BinaryOp::Add => a
                    .checked_add(*b)
                    .map_or(ConcreteValue::Top, ConcreteValue::Integer),
                BinaryOp::Sub => a
                    .checked_sub(*b)
                    .map_or(ConcreteValue::Top, ConcreteValue::Integer),
                BinaryOp::Mul => a
                    .checked_mul(*b)
                    .map_or(ConcreteValue::Top, ConcreteValue::Integer),
                BinaryOp::Div => {
                    if *b != 0 {
                        a.checked_div(*b)
                            .map_or(ConcreteValue::Top, ConcreteValue::Integer)
                    } else {
                        ConcreteValue::Top
                    }
                }
                BinaryOp::Lt => ConcreteValue::Boolean(a < b),
                BinaryOp::Le => ConcreteValue::Boolean(a <= b),
                BinaryOp::Gt => ConcreteValue::Boolean(a > b),
                BinaryOp::Ge => ConcreteValue::Boolean(a >= b),
                BinaryOp::Eq => ConcreteValue::Boolean(a == b),
                BinaryOp::Ne => ConcreteValue::Boolean(a != b),
                BinaryOp::And | BinaryOp::Or => ConcreteValue::Top,
            },
            (ConcreteValue::Boolean(a), ConcreteValue::Boolean(b)) => match op {
                BinaryOp::And => ConcreteValue::Boolean(*a && *b),
                BinaryOp::Or => ConcreteValue::Boolean(*a || *b),
                BinaryOp::Eq => ConcreteValue::Boolean(a == b),
                BinaryOp::Ne => ConcreteValue::Boolean(a != b),
                _ => ConcreteValue::Top,
            },
            _ => ConcreteValue::Top,
        }
    }
}

impl fmt::Display for ConcreteValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConcreteValue::Integer(n) => write!(f, "{n}"),
            ConcreteValue::Boolean(b) => write!(f, "{b}"),
            ConcreteValue::Unsigned(n) => write!(f, "{n}u"),
            ConcreteValue::Null => write!(f, "null"),
            ConcreteValue::Unknown => write!(f, "⊥"),
            ConcreteValue::Top => write!(f, "⊤"),
        }
    }
}

// ==================================================================================
// Binary Operations
// ==================================================================================

/// Binary operations for value tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    /// Addition
    Add,
    /// Subtraction
    Sub,
    /// Multiplication
    Mul,
    /// Division
    Div,
    /// Less than
    Lt,
    /// Less than or equal
    Le,
    /// Greater than
    Gt,
    /// Greater than or equal
    Ge,
    /// Equal
    Eq,
    /// Not equal
    Ne,
    /// Logical AND
    And,
    /// Logical OR
    Or,
}

// ==================================================================================
// Value Range Analysis
// ==================================================================================

/// Range of possible values for integers
///
/// Used for proving bounds constraints and refining escape analysis.
/// For example, if we know size ∈ [0, 99], we can prove small allocations
/// don't escape.
///
/// Integer range [min, max] for proving bounds constraints. Example: if
/// size is in [0, 99], the allocation is bounded and may qualify for
/// stack promotion (NoEscape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueRange {
    /// Minimum value (inclusive)
    pub min: i64,

    /// Maximum value (inclusive)
    pub max: i64,

    /// Whether range is definite (vs conservative estimate)
    pub definite: bool,
}

impl ValueRange {
    /// Create range from single constant
    #[must_use]
    pub fn from_constant(value: i64) -> Self {
        Self {
            min: value,
            max: value,
            definite: true,
        }
    }

    /// Create range from bounds
    #[must_use]
    pub fn from_bounds(min: i64, max: i64) -> Self {
        Self {
            min,
            max,
            definite: false,
        }
    }

    /// Create unbounded range (all possible values)
    #[must_use]
    pub fn unbounded() -> Self {
        Self {
            min: i64::MIN,
            max: i64::MAX,
            definite: false,
        }
    }

    /// Check if range contains a value
    #[must_use]
    pub fn contains(&self, value: i64) -> bool {
        self.min <= value && value <= self.max
    }

    /// Intersect two ranges
    #[must_use]
    pub fn intersect(&self, other: &ValueRange) -> ValueRange {
        ValueRange {
            min: self.min.max(other.min),
            max: self.max.min(other.max),
            definite: self.definite && other.definite,
        }
    }

    /// Union two ranges (conservative)
    #[must_use]
    pub fn union(&self, other: &ValueRange) -> ValueRange {
        ValueRange {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
            definite: false,
        }
    }

    /// Check if range is empty (impossible)
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min > self.max
    }

    /// Evaluate binary operation on ranges
    #[must_use]
    pub fn eval_binop(&self, op: BinaryOp, other: &ValueRange) -> Maybe<ValueRange> {
        match op {
            BinaryOp::Add => {
                let min = self.min.saturating_add(other.min);
                let max = self.max.saturating_add(other.max);
                Maybe::Some(ValueRange::from_bounds(min, max))
            }
            BinaryOp::Sub => {
                let min = self.min.saturating_sub(other.max);
                let max = self.max.saturating_sub(other.min);
                Maybe::Some(ValueRange::from_bounds(min, max))
            }
            BinaryOp::Mul => {
                // Conservative: compute all combinations
                let vals = [
                    self.min.saturating_mul(other.min),
                    self.min.saturating_mul(other.max),
                    self.max.saturating_mul(other.min),
                    self.max.saturating_mul(other.max),
                ];
                let min = *vals.iter().min().unwrap();
                let max = *vals.iter().max().unwrap();
                Maybe::Some(ValueRange::from_bounds(min, max))
            }
            _ => Maybe::None,
        }
    }
}

impl fmt::Display for ValueRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.min == self.max {
            write!(f, "{}", self.min)
        } else if self.definite {
            write!(f, "[{}, {}]", self.min, self.max)
        } else {
            write!(f, "[{}, {}]?", self.min, self.max)
        }
    }
}

// ==================================================================================
// Symbolic Values
// ==================================================================================

/// Symbolic value for complex expressions
///
/// Represents values that aren't known concretely but can be expressed
/// symbolically for constraint solving.
///
/// Values not known concretely but expressible symbolically for constraint solving.
/// Used to track relationships like `x = y + 1` through the CFG.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SymbolicValue {
    /// Concrete constant value
    Concrete(ConcreteValue),

    /// Symbolic variable (SSA version)
    Variable(u32),

    /// Binary operation on symbolic values
    BinaryOp {
        /// Operation
        op: BinaryOp,
        /// Left operand
        left: Box<SymbolicValue>,
        /// Right operand
        right: Box<SymbolicValue>,
    },

    /// Phi node (merge of multiple values)
    Phi {
        /// Block where phi occurs
        block: BlockId,
        /// Incoming values from predecessors
        incoming: List<(BlockId, SymbolicValue)>,
    },

    /// Unknown symbolic value
    Unknown,
}

impl SymbolicValue {
    /// Create symbolic value from concrete value
    #[must_use]
    pub fn from_concrete(value: ConcreteValue) -> Self {
        SymbolicValue::Concrete(value)
    }

    /// Create symbolic variable
    #[must_use]
    pub fn variable(ssa_version: u32) -> Self {
        SymbolicValue::Variable(ssa_version)
    }

    /// Create binary operation
    #[must_use]
    pub fn binop(op: BinaryOp, left: SymbolicValue, right: SymbolicValue) -> Self {
        SymbolicValue::BinaryOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Try to evaluate to concrete value
    #[must_use]
    pub fn evaluate(&self, env: &Map<u32, ConcreteValue>) -> ConcreteValue {
        match self {
            SymbolicValue::Concrete(v) => v.clone(),
            SymbolicValue::Variable(ssa) => env.get(ssa).cloned().unwrap_or(ConcreteValue::Unknown),
            SymbolicValue::BinaryOp { op, left, right } => {
                let l = left.evaluate(env);
                let r = right.evaluate(env);
                l.eval_binop(*op, &r)
            }
            SymbolicValue::Phi { incoming, .. } => {
                // Merge all incoming values
                let mut result = ConcreteValue::Unknown;
                for (_, val) in incoming {
                    let evaluated = val.evaluate(env);
                    result = result.merge(&evaluated);
                }
                result
            }
            SymbolicValue::Unknown => ConcreteValue::Unknown,
        }
    }

    /// Check if value is definitely true
    #[must_use]
    pub fn is_definitely_true(&self, env: &Map<u32, ConcreteValue>) -> bool {
        matches!(self.evaluate(env), ConcreteValue::Boolean(true))
    }

    /// Check if value is definitely false
    #[must_use]
    pub fn is_definitely_false(&self, env: &Map<u32, ConcreteValue>) -> bool {
        matches!(self.evaluate(env), ConcreteValue::Boolean(false))
    }
}

impl fmt::Display for SymbolicValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SymbolicValue::Concrete(v) => write!(f, "{v}"),
            SymbolicValue::Variable(ssa) => write!(f, "v{ssa}"),
            SymbolicValue::BinaryOp { op, left, right } => {
                write!(f, "({left} {op:?} {right})")
            }
            SymbolicValue::Phi { block, incoming } => {
                write!(f, "φ{}(", block.0)?;
                for (i, (blk, val)) in incoming.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}:{val}", blk.0)?;
                }
                write!(f, ")")
            }
            SymbolicValue::Unknown => write!(f, "?"),
        }
    }
}

// ==================================================================================
// Value State at Program Points
// ==================================================================================

/// Value state at a program point
///
/// Maps SSA versions to their known values (concrete or range).
/// Maintained via dataflow analysis through the CFG.
///
/// Maps SSA versions to concrete values and range constraints at a program point.
/// Updated via forward dataflow through the CFG, merged at join points.
#[derive(Debug, Clone)]
pub struct ValueState {
    /// Concrete values for SSA versions
    concrete_values: Map<u32, ConcreteValue>,

    /// Range constraints for SSA versions
    ranges: Map<u32, ValueRange>,

    /// Symbolic expressions
    symbolic: Map<u32, SymbolicValue>,
}

impl ValueState {
    /// Create empty value state
    #[must_use]
    pub fn new() -> Self {
        Self {
            concrete_values: Map::new(),
            ranges: Map::new(),
            symbolic: Map::new(),
        }
    }

    /// Set concrete value for SSA version
    pub fn set_concrete(&mut self, ssa_version: u32, value: ConcreteValue) {
        // Update range if it's an integer before inserting
        if let ConcreteValue::Integer(n) = value {
            self.ranges
                .insert(ssa_version, ValueRange::from_constant(n));
        }

        self.concrete_values.insert(ssa_version, value);
    }

    /// Get concrete value for SSA version
    #[must_use]
    pub fn get_concrete(&self, ssa_version: u32) -> Maybe<ConcreteValue> {
        self.concrete_values.get(&ssa_version).cloned()
    }

    /// Set range constraint for SSA version
    pub fn set_range(&mut self, ssa_version: u32, range: ValueRange) {
        self.ranges.insert(ssa_version, range);
    }

    /// Get range constraint for SSA version
    #[must_use]
    pub fn get_range(&self, ssa_version: u32) -> Maybe<ValueRange> {
        self.ranges.get(&ssa_version).cloned()
    }

    /// Set symbolic value
    pub fn set_symbolic(&mut self, ssa_version: u32, value: SymbolicValue) {
        self.symbolic.insert(ssa_version, value);
    }

    /// Get symbolic value
    #[must_use]
    pub fn get_symbolic(&self, ssa_version: u32) -> Maybe<SymbolicValue> {
        self.symbolic.get(&ssa_version).cloned()
    }

    /// Merge two value states (for phi nodes)
    #[must_use]
    pub fn merge(&self, other: &ValueState) -> ValueState {
        let mut result = ValueState::new();

        // Merge concrete values
        for (ssa, val) in &self.concrete_values {
            if let Maybe::Some(other_val) = other.concrete_values.get(ssa) {
                result.concrete_values.insert(*ssa, val.merge(other_val));
            } else {
                result.concrete_values.insert(*ssa, val.clone());
            }
        }

        // Add values only in other
        for (ssa, val) in &other.concrete_values {
            if !result.concrete_values.contains_key(ssa) {
                result.concrete_values.insert(*ssa, val.clone());
            }
        }

        // Merge ranges (conservative: take union)
        for (ssa, range) in &self.ranges {
            if let Maybe::Some(other_range) = other.ranges.get(ssa) {
                result.ranges.insert(*ssa, range.union(other_range));
            } else {
                result.ranges.insert(*ssa, range.clone());
            }
        }

        for (ssa, range) in &other.ranges {
            if !result.ranges.contains_key(ssa) {
                result.ranges.insert(*ssa, range.clone());
            }
        }

        result
    }

    /// Refine state with path condition
    ///
    /// When we know a predicate is true/false on a path, refine the value state.
    /// For example: if (x < 10) -> refine x to [MIN, 9]
    #[must_use]
    pub fn refine_with_condition(&self, predicate: &SymbolicValue, is_true: bool) -> ValueState {
        let mut refined = self.clone();

        // Try to extract range constraints from predicate
        if let SymbolicValue::BinaryOp { op, left, right } = predicate
            && let (
                SymbolicValue::Variable(var),
                SymbolicValue::Concrete(ConcreteValue::Integer(n)),
            ) = (left.as_ref(), right.as_ref())
        {
            let constraint = match (op, is_true) {
                (BinaryOp::Lt, true) => Maybe::Some(ValueRange::from_bounds(i64::MIN, n - 1)),
                (BinaryOp::Le, true) => Maybe::Some(ValueRange::from_bounds(i64::MIN, *n)),
                (BinaryOp::Gt, true) => Maybe::Some(ValueRange::from_bounds(n + 1, i64::MAX)),
                (BinaryOp::Ge, true) => Maybe::Some(ValueRange::from_bounds(*n, i64::MAX)),
                (BinaryOp::Eq, true) => Maybe::Some(ValueRange::from_constant(*n)),
                (BinaryOp::Lt, false) => Maybe::Some(ValueRange::from_bounds(*n, i64::MAX)),
                (BinaryOp::Le, false) => Maybe::Some(ValueRange::from_bounds(n + 1, i64::MAX)),
                (BinaryOp::Gt, false) => Maybe::Some(ValueRange::from_bounds(i64::MIN, *n)),
                (BinaryOp::Ge, false) => Maybe::Some(ValueRange::from_bounds(i64::MIN, n - 1)),
                (BinaryOp::Ne, true) => Maybe::None, // Can't refine with inequality
                _ => Maybe::None,
            };

            if let Maybe::Some(new_range) = constraint {
                if let Maybe::Some(existing) = refined.get_range(*var) {
                    refined.set_range(*var, existing.intersect(&new_range));
                } else {
                    refined.set_range(*var, new_range);
                }
            }
        }

        refined
    }
}

impl Default for ValueState {
    fn default() -> Self {
        Self::new()
    }
}

// ==================================================================================
// Value Propagator
// ==================================================================================

/// Statistics for value propagation
#[derive(Debug, Clone, Default)]
pub struct PropagationStats {
    /// Number of concrete values propagated
    pub concrete_propagated: usize,

    /// Number of ranges refined
    pub ranges_refined: usize,

    /// Number of symbolic expressions created
    pub symbolic_created: usize,

    /// Number of phi nodes processed
    pub phi_nodes: usize,

    /// Number of dataflow iterations
    pub iterations: usize,
}

/// Value propagator through CFG
///
/// Implements dataflow analysis to propagate values through the control
/// flow graph. Computes value state at each program point.
///
/// # Algorithm
/// 1. Initialize entry block with parameter values
/// 2. Iterate CFG in topological order
/// 3. For each block:
///    - Merge incoming states from predecessors
///    - Apply transfer function (evaluate operations)
///    - Propagate to successors
/// 4. Fixed point reached when no changes occur
///
/// Forward dataflow propagation engine: iterates CFG in topological order,
/// merges incoming states at join points, applies transfer functions (evaluate
/// operations), and propagates to successors until fixpoint.
#[derive(Debug)]
pub struct ValuePropagator {
    /// Value state at entry of each block
    block_entry_states: Map<BlockId, ValueState>,

    /// Value state at exit of each block
    block_exit_states: Map<BlockId, ValueState>,

    /// Propagation statistics
    stats: PropagationStats,
}

impl ValuePropagator {
    /// Create new value propagator
    #[must_use]
    pub fn new() -> Self {
        Self {
            block_entry_states: Map::new(),
            block_exit_states: Map::new(),
            stats: PropagationStats::default(),
        }
    }

    /// Get value state at block entry
    #[must_use]
    pub fn get_entry_state(&self, block: BlockId) -> Maybe<&ValueState> {
        self.block_entry_states.get(&block)
    }

    /// Get value state at block exit
    #[must_use]
    pub fn get_exit_state(&self, block: BlockId) -> Maybe<&ValueState> {
        self.block_exit_states.get(&block)
    }

    /// Set entry state for block
    pub fn set_entry_state(&mut self, block: BlockId, state: ValueState) {
        self.block_entry_states.insert(block, state);
    }

    /// Merge incoming states from predecessors
    #[must_use]
    pub fn merge_predecessor_states(&self, predecessors: &Set<BlockId>) -> ValueState {
        let mut result = ValueState::new();

        for pred_id in predecessors {
            if let Maybe::Some(pred_state) = self.get_exit_state(*pred_id) {
                result = result.merge(pred_state);
                self.stats.phi_nodes;
            }
        }

        result
    }

    /// Propagate constant assignment: x = c
    pub fn propagate_constant(
        &mut self,
        state: &mut ValueState,
        ssa_version: u32,
        value: ConcreteValue,
    ) {
        state.set_concrete(ssa_version, value.clone());
        state.set_symbolic(ssa_version, SymbolicValue::from_concrete(value));
        self.stats.concrete_propagated += 1;
    }

    /// Propagate binary operation: x = a op b
    pub fn propagate_binop(
        &mut self,
        state: &mut ValueState,
        ssa_version: u32,
        op: BinaryOp,
        left_ssa: u32,
        right_ssa: u32,
    ) {
        // Try concrete evaluation first
        if let (Maybe::Some(left_val), Maybe::Some(right_val)) =
            (state.get_concrete(left_ssa), state.get_concrete(right_ssa))
        {
            let result = left_val.eval_binop(op, &right_val);
            state.set_concrete(ssa_version, result.clone());
            state.set_symbolic(ssa_version, SymbolicValue::from_concrete(result));
            self.stats.concrete_propagated += 1;
            return;
        }

        // Try range evaluation
        if let (Maybe::Some(left_range), Maybe::Some(right_range)) =
            (state.get_range(left_ssa), state.get_range(right_ssa))
            && let Maybe::Some(result_range) = left_range.eval_binop(op, &right_range)
        {
            state.set_range(ssa_version, result_range);
            self.stats.ranges_refined += 1;
        }

        // Create symbolic expression
        let left_sym = state
            .get_symbolic(left_ssa)
            .unwrap_or(SymbolicValue::variable(left_ssa));
        let right_sym = state
            .get_symbolic(right_ssa)
            .unwrap_or(SymbolicValue::variable(right_ssa));

        state.set_symbolic(ssa_version, SymbolicValue::binop(op, left_sym, right_sym));
        self.stats.symbolic_created += 1;
    }

    /// Propagate phi node: x = φ(x1, x2, ...)
    pub fn propagate_phi(
        &mut self,
        state: &mut ValueState,
        ssa_version: u32,
        block: BlockId,
        incoming: &List<(BlockId, u32)>,
    ) {
        let mut concrete_values = List::new();
        let mut symbolic_incoming = List::new();

        for (pred_block, pred_ssa) in incoming {
            // Get predecessor exit state
            if let Maybe::Some(pred_state) = self.get_exit_state(*pred_block) {
                if let Maybe::Some(val) = pred_state.get_concrete(*pred_ssa) {
                    concrete_values.push(val);
                }

                let sym = pred_state
                    .get_symbolic(*pred_ssa)
                    .unwrap_or(SymbolicValue::variable(*pred_ssa));
                symbolic_incoming.push((*pred_block, sym));
            }
        }

        // Merge concrete values
        if !concrete_values.is_empty() {
            let mut merged = concrete_values[0].clone();
            for val in concrete_values.iter().skip(1) {
                merged = merged.merge(val);
            }
            state.set_concrete(ssa_version, merged);
        }

        // Create phi symbolic value
        state.set_symbolic(
            ssa_version,
            SymbolicValue::Phi {
                block,
                incoming: symbolic_incoming,
            },
        );

        self.stats.phi_nodes += 1;
    }

    /// Get propagation statistics
    #[must_use]
    pub fn stats(&self) -> &PropagationStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = PropagationStats::default();
    }
}

impl Default for ValuePropagator {
    fn default() -> Self {
        Self::new()
    }
}

// ==================================================================================
// Path Predicate Evaluation
// ==================================================================================

/// Path predicate for conditional branches
///
/// Represents a boolean condition that determines which path is taken.
/// Used to refine value states and prove path feasibility.
#[derive(Debug, Clone)]
pub struct PathPredicate {
    /// Symbolic value of the predicate
    pub condition: SymbolicValue,

    /// True if this is the true-branch, false for false-branch
    pub is_true_branch: bool,

    /// Block where condition is evaluated
    pub block: BlockId,
}

impl PathPredicate {
    /// Create new path predicate
    #[must_use]
    pub fn new(condition: SymbolicValue, is_true_branch: bool, block: BlockId) -> Self {
        Self {
            condition,
            is_true_branch,
            block,
        }
    }

    /// Evaluate predicate with value state
    #[must_use]
    pub fn evaluate(&self, state: &ValueState) -> Maybe<bool> {
        let value = self.condition.evaluate(&state.concrete_values);

        match value {
            ConcreteValue::Boolean(b) => Maybe::Some(if self.is_true_branch { b } else { !b }),
            _ => Maybe::None,
        }
    }

    /// Check if predicate is definitely satisfiable
    #[must_use]
    pub fn is_satisfiable(&self, state: &ValueState) -> bool {
        match self.evaluate(state) {
            Maybe::Some(true) => true,
            Maybe::Some(false) => false,
            Maybe::None => true, // Unknown, conservatively assume satisfiable
        }
    }

    /// Refine value state based on this predicate
    #[must_use]
    pub fn refine_state(&self, state: &ValueState) -> ValueState {
        state.refine_with_condition(&self.condition, self.is_true_branch)
    }
}

// ==================================================================================
// Public API for Integration
// ==================================================================================

/// Value tracking configuration
#[derive(Debug, Clone)]
pub struct ValueTrackingConfig {
    /// Enable constant propagation
    pub enable_constant_propagation: bool,

    /// Enable range analysis
    pub enable_range_analysis: bool,

    /// Enable symbolic execution
    pub enable_symbolic_execution: bool,

    /// Maximum iterations for fixed point
    pub max_iterations: usize,
}

impl Default for ValueTrackingConfig {
    fn default() -> Self {
        Self {
            enable_constant_propagation: true,
            enable_range_analysis: true,
            enable_symbolic_execution: true,
            max_iterations: 100,
        }
    }
}

/// Value tracking result
#[derive(Debug, Clone)]
pub struct ValueTrackingResult {
    /// Value state at each block
    pub block_states: Map<BlockId, ValueState>,

    /// Infeasible paths detected
    pub infeasible_paths: Set<List<BlockId>>,

    /// Statistics
    pub stats: PropagationStats,
}

impl ValueTrackingResult {
    /// Create new result
    #[must_use]
    pub fn new() -> Self {
        Self {
            block_states: Map::new(),
            infeasible_paths: Set::new(),
            stats: PropagationStats::default(),
        }
    }

    /// Get value state for block
    #[must_use]
    pub fn get_state(&self, block: BlockId) -> Maybe<&ValueState> {
        self.block_states.get(&block)
    }

    /// Check if path is feasible
    #[must_use]
    pub fn is_path_feasible(&self, path: &List<BlockId>) -> bool {
        !self.infeasible_paths.contains(path)
    }
}

impl Default for ValueTrackingResult {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_concrete_value_merge() {
        let v1 = ConcreteValue::Integer(42);
        let v2 = ConcreteValue::Integer(42);
        assert_eq!(v1.merge(&v2), ConcreteValue::Integer(42));

        let v3 = ConcreteValue::Integer(10);
        assert_eq!(v1.merge(&v3), ConcreteValue::Top);
    }

    #[test]
    fn test_value_range_intersect() {
        let r1 = ValueRange::from_bounds(0, 10);
        let r2 = ValueRange::from_bounds(5, 15);
        let r3 = r1.intersect(&r2);

        assert_eq!(r3.min, 5);
        assert_eq!(r3.max, 10);
    }

    #[test]
    fn test_symbolic_evaluation() {
        let sym = SymbolicValue::binop(
            BinaryOp::Add,
            SymbolicValue::variable(0),
            SymbolicValue::from_concrete(ConcreteValue::Integer(10)),
        );

        let mut env = Map::new();
        env.insert(0, ConcreteValue::Integer(32));

        assert_eq!(sym.evaluate(&env), ConcreteValue::Integer(42));
    }
}
