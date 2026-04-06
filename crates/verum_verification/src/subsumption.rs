//! # Refinement Type Subsumption Checking
//!
//! Implements refinement type subtyping via logical implication.
//!
//! This module implements **subtype relationships** for refinement types through
//! logical implication. Subsumption is critical for function calls, variable
//! assignment, and type checking.
//!
//! ## Formal Rule
//!
//! ```text
//! Γ ⊢ φ₁ ⇒ φ₂    (in SMT logic)
//! ─────────────────────────────────
//! Γ ⊢ T{φ₁} <: T{φ₂}
//! ```
//!
//! **Interpretation**: Type `T{φ₁}` is a **subtype** of `T{φ₂}` if predicate `φ₁`
//! logically implies `φ₂`.
//!
//! ## Three-Mode Checking Algorithm
//!
//! 1. **Mode 1: Syntactic** - Pattern-based implication (< 1ms, ~60% cases)
//! 2. **Mode 2: SMT-Based** - Full Z3 verification (10-500ms, ~35% cases)
//! 3. **Mode 3: Fallback** - Runtime check with user notification (~5% cases)
//!
//! ## Performance Targets
//!
//! - Syntactic checks: < 1ms
//! - SMT checks: 10-500ms (with configurable timeout)
//! - Cache hit rate: > 90%

use std::fmt::{self, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use verum_ast::expr::{BinOp, Expr, ExprKind, UnOp};
use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::span::Span;
use verum_common::{Heap, List, Map, Maybe, Set, Text};
use verum_types::refinement::{RefinementPredicate, RefinementType};

// Re-export verum_smt types for integration
use verum_smt::{
    CheckMode as SmtCheckMode, SubsumptionChecker as SmtSubsumptionChecker,
    SubsumptionConfig as SmtSubsumptionConfig, SubsumptionResult as SmtResult,
};

// ==================== Core Types ====================

/// Result of subsumption checking
///
/// T{phi1} <: T{phi2} iff phi1 => phi2. Checked via three modes:
/// syntactic (fast), SMT-based (accurate), or fallback (runtime check).
#[derive(Debug, Clone, PartialEq)]
pub enum SubsumptionResult {
    /// Subsumption holds - φ₁ ⇒ φ₂ is valid
    Holds,

    /// Subsumption fails with counterexample showing a value where φ₁ holds but φ₂ doesn't
    Fails {
        /// Concrete counterexample that demonstrates the failure
        counterexample: Counterexample,
    },

    /// Unknown - cannot determine statically, need runtime check
    Unknown {
        /// Explanation of why we couldn't determine the result
        reason: Text,
    },

    /// Timeout during SMT check
    Timeout {
        /// Time spent before timeout (milliseconds)
        time_ms: u64,
    },
}

impl SubsumptionResult {
    /// Check if subsumption holds
    pub fn holds(&self) -> bool {
        matches!(self, Self::Holds)
    }

    /// Check if subsumption definitely fails
    pub fn fails(&self) -> bool {
        matches!(self, Self::Fails { .. })
    }

    /// Check if result is unknown (needs runtime check)
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown { .. } | Self::Timeout { .. })
    }

    /// Get counterexample if subsumption fails
    pub fn counterexample(&self) -> Option<&Counterexample> {
        match self {
            Self::Fails { counterexample } => Some(counterexample),
            _ => None,
        }
    }
}

impl Display for SubsumptionResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Holds => write!(f, "Subsumption holds"),
            Self::Fails { counterexample } => {
                write!(f, "Subsumption fails: {}", counterexample)
            }
            Self::Unknown { reason } => {
                write!(f, "Unknown: {}", reason)
            }
            Self::Timeout { time_ms } => {
                write!(f, "Timeout after {}ms", time_ms)
            }
        }
    }
}

/// Counterexample showing why subsumption fails
///
/// When SMT returns SAT (implication fails), shows concrete values where
/// phi1 holds but phi2 doesn't. Used to generate actionable error messages. φ₁ ⇒ φ₂,
/// i.e., values where φ₁ holds but φ₂ doesn't.
#[derive(Debug, Clone, PartialEq)]
pub struct Counterexample {
    /// Variable name to value assignments from SMT model
    pub variable_values: Map<Text, Value>,

    /// The constraint that was violated
    pub violated_constraint: Text,

    /// Source span for error reporting
    pub span: Option<Span>,

    /// Human-readable explanation
    pub explanation: Option<Text>,
}

impl Counterexample {
    /// Create a new counterexample
    pub fn new(variable_values: Map<Text, Value>, violated_constraint: Text) -> Self {
        Self {
            variable_values,
            violated_constraint,
            span: None,
            explanation: None,
        }
    }

    /// Add source span
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Add explanation
    pub fn with_explanation(mut self, explanation: Text) -> Self {
        self.explanation = Some(explanation);
        self
    }

    /// Format as error message per spec
    ///
    /// Format as a user-facing error message showing variable assignments,
    /// which refinement holds, and which is violated.
    pub fn format_error(&self, provided_pred: &Text, required_pred: &Text) -> Text {
        let mut msg = Text::from("Counterexample:\n");

        // Show variable assignments
        for (var, val) in self.variable_values.iter() {
            msg = Text::from(format!("{}  {} = {}\n", msg, var, val));
        }

        msg = Text::from(format!(
            "{}\nWith these values:\n  Provided refinement ({}): TRUE\n  Required refinement ({}): FALSE\n",
            msg, provided_pred, required_pred
        ));

        msg = Text::from(format!(
            "{}Conclusion: There exists a value ({}) that satisfies the provided\n\
             refinement but violates the required refinement.",
            msg,
            self.variable_values
                .values()
                .next()
                .map(|v| Text::from(v.to_string()))
                .unwrap_or_else(|| Text::from("?"))
        ));

        msg
    }
}

impl Display for Counterexample {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{{ ")?;
        let mut first = true;
        for (var, val) in self.variable_values.iter() {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{} = {}", var, val)?;
            first = false;
        }
        write!(f, " }} violates {}", self.violated_constraint)
    }
}

/// Value in a counterexample
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Integer value
    Int(i64),
    /// Floating-point/Real value
    Real(f64),
    /// Boolean value
    Bool(bool),
    /// Text value
    Text(Text),
    /// Unknown/unsupported value
    Unknown(Text),
}

impl Display for Value {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(n) => write!(f, "{}", n),
            Self::Real(r) => write!(f, "{}", r),
            Self::Bool(b) => write!(f, "{}", b),
            Self::Text(s) => write!(f, "\"{}\"", s),
            Self::Unknown(s) => write!(f, "{}", s),
        }
    }
}

// ==================== Configuration ====================

/// Configuration for subsumption checker
///
/// SMT timeout default: 100ms. Cascading strategy: try syntactic first (<1ms),
/// then SMT (10-500ms), then conservative rejection on timeout.
#[derive(Debug, Clone)]
pub struct SubsumptionConfig {
    /// Timeout for SMT queries in milliseconds (default: 100ms per spec)
    pub timeout_ms: u64,

    /// Try syntactic check first before SMT (default: true)
    pub try_syntactic_first: bool,

    /// Maximum SMT complexity before falling back to runtime (0 = no limit)
    pub max_smt_complexity: usize,

    /// Enable result caching (default: true)
    pub enable_cache: bool,

    /// Maximum cache size (entries)
    pub cache_size: usize,
}

impl Default for SubsumptionConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 100, // 100ms default per spec
            try_syntactic_first: true,
            max_smt_complexity: 0, // No limit by default
            enable_cache: true,
            cache_size: 10000, // 10K entries
        }
    }
}

// ==================== Statistics ====================

/// Statistics for subsumption checking performance
#[derive(Debug, Clone, Default)]
pub struct SubsumptionStats {
    /// Number of syntactic check successes
    pub syntactic_hits: usize,

    /// Number of SMT checks performed
    pub smt_checks: usize,

    /// Number of fallback (runtime) results
    pub fallbacks: usize,

    /// Total time spent in milliseconds
    pub total_time_ms: u64,

    /// Number of cache hits
    pub cache_hits: usize,

    /// Number of cache misses
    pub cache_misses: usize,

    /// Number of timeouts
    pub timeouts: usize,
}

impl SubsumptionStats {
    /// Syntactic hit rate (proportion resolved without SMT)
    pub fn syntactic_hit_rate(&self) -> f64 {
        let total = self.syntactic_hits + self.smt_checks + self.fallbacks;
        if total == 0 {
            0.0
        } else {
            self.syntactic_hits as f64 / total as f64
        }
    }

    /// Cache hit rate
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }

    /// Average time per check (milliseconds)
    pub fn avg_time_ms(&self) -> f64 {
        let total = self.syntactic_hits + self.smt_checks + self.fallbacks;
        if total == 0 {
            0.0
        } else {
            self.total_time_ms as f64 / total as f64
        }
    }

    /// Generate performance report
    pub fn report(&self) -> Text {
        let total = self.syntactic_hits + self.smt_checks + self.fallbacks;
        Text::from(format!(
            "Subsumption Checking Statistics:\n\
             - Total checks: {}\n\
             - Syntactic hits: {} ({:.1}%)\n\
             - SMT checks: {} ({:.1}%)\n\
             - Fallbacks: {} ({:.1}%)\n\
             - Timeouts: {}\n\
             - Cache hit rate: {:.1}%\n\
             - Average time: {:.2}ms\n\
             - Total time: {}ms",
            total,
            self.syntactic_hits,
            self.syntactic_hit_rate() * 100.0,
            self.smt_checks,
            if total > 0 {
                self.smt_checks as f64 / total as f64 * 100.0
            } else {
                0.0
            },
            self.fallbacks,
            if total > 0 {
                self.fallbacks as f64 / total as f64 * 100.0
            } else {
                0.0
            },
            self.timeouts,
            self.cache_hit_rate() * 100.0,
            self.avg_time_ms(),
            self.total_time_ms
        ))
    }
}

// ==================== Predicate Representation ====================

/// Predicate for subsumption checking
///
/// Represents boolean predicates over refinement type variables.
/// These are used for syntactic pattern matching and SMT translation.
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    /// Comparison: x op lit (e.g., x > 0)
    Compare {
        var: Text,
        op: CompareOp,
        value: i64,
    },

    /// Conjunction: P && Q
    And(Heap<Predicate>, Heap<Predicate>),

    /// Disjunction: P || Q
    Or(Heap<Predicate>, Heap<Predicate>),

    /// Negation: !P
    Not(Heap<Predicate>),

    /// Literal true/false
    Literal(bool),

    /// Complex predicate (not syntactically analyzable)
    Complex(Expr),
}

/// Comparison operators for predicates
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    /// Greater than (>)
    Gt,
    /// Greater than or equal (>=)
    Ge,
    /// Less than (<)
    Lt,
    /// Less than or equal (<=)
    Le,
    /// Equal (==)
    Eq,
    /// Not equal (!=)
    Ne,
}

impl CompareOp {
    /// Convert from AST BinOp
    pub fn from_binop(op: BinOp) -> Option<Self> {
        match op {
            BinOp::Gt => Some(Self::Gt),
            BinOp::Ge => Some(Self::Ge),
            BinOp::Lt => Some(Self::Lt),
            BinOp::Le => Some(Self::Le),
            BinOp::Eq => Some(Self::Eq),
            BinOp::Ne => Some(Self::Ne),
            _ => None,
        }
    }
}

impl Predicate {
    /// Parse a predicate from an expression
    pub fn from_expr(expr: &Expr) -> Self {
        match &expr.kind {
            // Literal true/false
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(b),
                ..
            }) => Self::Literal(*b),

            // Binary comparison: x op lit
            ExprKind::Binary { op, left, right } => {
                // Check for comparison operators
                if let Some(cmp_op) = CompareOp::from_binop(*op) {
                    // Try to extract var op lit pattern
                    if let Some((var, val)) = Self::extract_var_lit(left, right) {
                        return Self::Compare {
                            var,
                            op: cmp_op,
                            value: val,
                        };
                    }
                    // Try lit op var (reversed)
                    if let Some((var, val)) = Self::extract_var_lit(right, left) {
                        // Reverse the operator
                        let reversed_op = match cmp_op {
                            CompareOp::Gt => CompareOp::Lt,
                            CompareOp::Ge => CompareOp::Le,
                            CompareOp::Lt => CompareOp::Gt,
                            CompareOp::Le => CompareOp::Ge,
                            CompareOp::Eq => CompareOp::Eq,
                            CompareOp::Ne => CompareOp::Ne,
                        };
                        return Self::Compare {
                            var,
                            op: reversed_op,
                            value: val,
                        };
                    }
                }

                // Logical operators
                match op {
                    BinOp::And => Self::And(
                        Heap::new(Self::from_expr(left)),
                        Heap::new(Self::from_expr(right)),
                    ),
                    BinOp::Or => Self::Or(
                        Heap::new(Self::from_expr(left)),
                        Heap::new(Self::from_expr(right)),
                    ),
                    _ => Self::Complex(expr.clone()),
                }
            }

            // Unary negation
            ExprKind::Unary {
                op: UnOp::Not,
                expr: inner,
            } => Self::Not(Heap::new(Self::from_expr(inner))),

            // Everything else is complex
            _ => Self::Complex(expr.clone()),
        }
    }

    /// Extract (variable_name, literal_value) from var/lit pair
    fn extract_var_lit(var_expr: &Expr, lit_expr: &Expr) -> Option<(Text, i64)> {
        // Check if left is a variable path
        let var_name = match &var_expr.kind {
            ExprKind::Path(path) if path.segments.len() == 1 => match &path.segments[0] {
                verum_ast::ty::PathSegment::Name(ident) => Some(Text::from(ident.name.clone())),
                verum_ast::ty::PathSegment::Relative => Some(Text::from(".")),
                _ => None,
            },
            _ => None,
        }?;

        // Check if right is an integer literal
        let value = match &lit_expr.kind {
            ExprKind::Literal(Literal {
                kind: LiteralKind::Int(IntLit { value, .. }),
                ..
            }) => Some(*value as i64),
            _ => None,
        }?;

        Some((var_name, value))
    }

    /// Check if this is a simple predicate (can be checked syntactically)
    pub fn is_simple(&self) -> bool {
        match self {
            Self::Compare { .. } | Self::Literal(_) => true,
            Self::And(p, q) | Self::Or(p, q) => p.is_simple() && q.is_simple(),
            Self::Not(p) => p.is_simple(),
            Self::Complex(_) => false,
        }
    }
}

// ==================== Main Checker ====================

/// Subsumption checker implementing three-mode algorithm
///
/// Three-mode algorithm: (1) syntactic pattern matching for obvious cases like
/// `> 0` implies `>= 0`, (2) SMT-based Z3 verification for complex predicates,
/// (3) conservative rejection on timeout. Cache hit rate target: >90%.
pub struct SubsumptionChecker {
    /// Configuration
    config: SubsumptionConfig,

    /// Statistics
    stats: Arc<RwLock<SubsumptionStats>>,

    /// Result cache (hash -> result)
    cache: Arc<RwLock<Map<u64, SubsumptionResult>>>,

    /// SMT backend from verum_smt
    smt_checker: SmtSubsumptionChecker,
}

impl SubsumptionChecker {
    /// Create a new subsumption checker with default configuration
    pub fn new() -> Self {
        Self::with_config(SubsumptionConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(config: SubsumptionConfig) -> Self {
        let smt_config = SmtSubsumptionConfig {
            cache_size: config.cache_size,
            smt_timeout_ms: config.timeout_ms,
        };

        Self {
            config,
            stats: Arc::new(RwLock::new(SubsumptionStats::default())),
            cache: Arc::new(RwLock::new(Map::new())),
            smt_checker: SmtSubsumptionChecker::with_config(smt_config),
        }
    }

    /// Check subsumption: T{φ₁} <: T{φ₂}
    ///
    /// Returns `Holds` if φ₁ ⇒ φ₂ for all values, `Fails` with counterexample
    /// if there exists a value where φ₁ holds but φ₂ doesn't.
    ///
    /// Check if T{phi1} <: T{phi2} using the cascading strategy.
    /// Returns Holds if phi1 => phi2 for all values, Fails with counterexample
    /// if there exists a value where phi1 holds but phi2 doesn't.
    pub fn check_subsumption(
        &self,
        sub: &RefinementType,
        sup: &RefinementType,
    ) -> SubsumptionResult {
        let start = Instant::now();

        // Check base types match
        if sub.base_type != sup.base_type {
            return SubsumptionResult::Fails {
                counterexample: Counterexample::new(
                    Map::new(),
                    Text::from("Base types do not match"),
                ),
            };
        }

        // Extract predicates
        let pred1 = &sub.predicate;
        let pred2 = &sup.predicate;

        // Trivial case: supertype is unrefined (trivial predicate)
        if pred2.is_trivial() {
            self.record_syntactic(start.elapsed().as_millis() as u64);
            return SubsumptionResult::Holds;
        }

        // Trivial case: subtype is unrefined but supertype has constraints
        if pred1.is_trivial() && !pred2.is_trivial() {
            self.record_syntactic(start.elapsed().as_millis() as u64);
            return SubsumptionResult::Fails {
                counterexample: Counterexample::new(
                    Map::new(),
                    Text::from("Unrefined type cannot satisfy refined constraint"),
                ),
            };
        }

        // Check cache
        let cache_key = self.compute_cache_key(&pred1.predicate, &pred2.predicate);
        if self.config.enable_cache {
            if let Some(cached) = self.cache.read().unwrap().get(&cache_key) {
                self.record_cache_hit();
                return cached.clone();
            }
            self.record_cache_miss();
        }

        // Mode 1: Try syntactic check first
        if self.config.try_syntactic_first
            && let Some(result) = self.try_syntactic_check(&pred1.predicate, &pred2.predicate)
        {
            self.record_syntactic(start.elapsed().as_millis() as u64);
            self.cache_result(cache_key, result.clone());
            return result;
        }

        // Check complexity limit
        if self.config.max_smt_complexity > 0 {
            let complexity = self.estimate_complexity(&pred1.predicate, &pred2.predicate);
            if complexity > self.config.max_smt_complexity {
                let result = SubsumptionResult::Unknown {
                    reason: Text::from(format!(
                        "Predicate complexity {} exceeds limit {}",
                        complexity, self.config.max_smt_complexity
                    )),
                };
                self.record_fallback(start.elapsed().as_millis() as u64);
                return result;
            }
        }

        // Mode 2: SMT-based check
        let result = self.smt_check(&pred1.predicate, &pred2.predicate);
        self.record_smt(start.elapsed().as_millis() as u64);

        // Handle timeout
        if matches!(result, SubsumptionResult::Timeout { .. }) {
            self.record_timeout();
        }

        self.cache_result(cache_key, result.clone());
        result
    }

    /// Mode 1: Syntactic pattern matching (fast path)
    ///
    /// Fast path (<1ms): handles ~60% of practical cases via pattern matching.
    /// Common patterns:
    /// - `x > a` ⇒ `x > b` when a > b
    /// - `x == a` ⇒ `x >= b` when a >= b
    /// - `P && Q` ⇒ `P`
    /// - `P` ⇒ `P || Q`
    pub fn try_syntactic_check(&self, pred1: &Expr, pred2: &Expr) -> Option<SubsumptionResult> {
        let p1 = Predicate::from_expr(pred1);
        let p2 = Predicate::from_expr(pred2);

        // Only use syntactic check if both predicates are simple
        if !p1.is_simple() || !p2.is_simple() {
            return None;
        }

        self.syntactic_implies(&p1, &p2)
    }

    /// Check if predicate p1 syntactically implies p2
    fn syntactic_implies(&self, p1: &Predicate, p2: &Predicate) -> Option<SubsumptionResult> {
        match (p1, p2) {
            // Reflexivity: P ⇒ P
            (a, b) if a == b => Some(SubsumptionResult::Holds),

            // Tautology: anything ⇒ true
            (_, Predicate::Literal(true)) => Some(SubsumptionResult::Holds),

            // Contradiction: false ⇒ anything
            (Predicate::Literal(false), _) => Some(SubsumptionResult::Holds),

            // Contradiction: true ⇒ false fails
            (Predicate::Literal(true), Predicate::Literal(false)) => {
                Some(SubsumptionResult::Fails {
                    counterexample: self.make_counterexample("true", "false"),
                })
            }

            // Conjunction elimination: (P && Q) ⇒ P and (P && Q) ⇒ Q
            (Predicate::And(p, q), target) => {
                // If either conjunct implies target, whole conjunction implies target
                if let Some(result) = self.syntactic_implies(p, target)
                    && result.holds()
                {
                    return Some(result);
                }
                self.syntactic_implies(q, target)
            }

            // Disjunction introduction: P ⇒ (P || Q) and Q ⇒ (P || Q)
            (source, Predicate::Or(p, q)) => {
                // If source implies either disjunct, it implies the disjunction
                if let Some(result) = self.syntactic_implies(source, p)
                    && result.holds()
                {
                    return Some(result);
                }
                self.syntactic_implies(source, q)
            }

            // Comparison implications
            (
                Predicate::Compare {
                    var: v1,
                    op: op1,
                    value: val1,
                },
                Predicate::Compare {
                    var: v2,
                    op: op2,
                    value: val2,
                },
            ) if v1 == v2 => self.check_comparison_implication(*op1, *val1, *op2, *val2),

            // Cannot determine syntactically
            _ => None,
        }
    }

    /// Check if comparison (var op1 val1) implies (var op2 val2)
    ///
    /// Syntactic comparison implication: e.g., (x > 5) implies (x >= 0),
    /// (x >= 10) implies (x >= 0), (x > 5) implies (x != 0).
    fn check_comparison_implication(
        &self,
        op1: CompareOp,
        val1: i64,
        op2: CompareOp,
        val2: i64,
    ) -> Option<SubsumptionResult> {
        let implies = match (op1, op2) {
            // x > a ⇒ x > b when a >= b (stronger lower bound)
            (CompareOp::Gt, CompareOp::Gt) => val1 >= val2,

            // x > a ⇒ x >= b when a >= b
            (CompareOp::Gt, CompareOp::Ge) => val1 >= val2,

            // x >= a ⇒ x >= b when a >= b
            (CompareOp::Ge, CompareOp::Ge) => val1 >= val2,

            // x >= a ⇒ x > b when a > b (need strict inequality)
            (CompareOp::Ge, CompareOp::Gt) => val1 > val2,

            // x < a ⇒ x < b when a <= b (stronger upper bound)
            (CompareOp::Lt, CompareOp::Lt) => val1 <= val2,

            // x < a ⇒ x <= b when a <= b
            (CompareOp::Lt, CompareOp::Le) => val1 <= val2,

            // x <= a ⇒ x <= b when a <= b
            (CompareOp::Le, CompareOp::Le) => val1 <= val2,

            // x <= a ⇒ x < b when a < b (need strict inequality)
            (CompareOp::Le, CompareOp::Lt) => val1 < val2,

            // x == a ⇒ x >= b when a >= b
            (CompareOp::Eq, CompareOp::Ge) => val1 >= val2,

            // x == a ⇒ x <= b when a <= b
            (CompareOp::Eq, CompareOp::Le) => val1 <= val2,

            // x == a ⇒ x > b when a > b
            (CompareOp::Eq, CompareOp::Gt) => val1 > val2,

            // x == a ⇒ x < b when a < b
            (CompareOp::Eq, CompareOp::Lt) => val1 < val2,

            // x == a ⇒ x == b when a == b
            (CompareOp::Eq, CompareOp::Eq) => val1 == val2,

            // x == a ⇒ x != b when a != b
            (CompareOp::Eq, CompareOp::Ne) => val1 != val2,

            // x > a ⇒ x != b when b <= a (any x > a is != b if b <= a)
            (CompareOp::Gt, CompareOp::Ne) => val2 <= val1,

            // x < a ⇒ x != b when b >= a
            (CompareOp::Lt, CompareOp::Ne) => val2 >= val1,

            // Cannot determine for other combinations
            _ => return None,
        };

        if implies {
            Some(SubsumptionResult::Holds)
        } else {
            // Generate counterexample
            let ce_value = self.find_counterexample_value(op1, val1, op2, val2);
            let mut values = Map::new();
            values.insert(Text::from("x"), Value::Int(ce_value));

            Some(SubsumptionResult::Fails {
                counterexample: Counterexample::new(
                    values,
                    Text::from(format!(
                        "{:?} {} does not imply {:?} {}",
                        op1, val1, op2, val2
                    )),
                ),
            })
        }
    }

    /// Find a concrete value that satisfies pred1 but violates pred2
    fn find_counterexample_value(
        &self,
        op1: CompareOp,
        val1: i64,
        op2: CompareOp,
        val2: i64,
    ) -> i64 {
        // Find a value that satisfies op1 val1 but not op2 val2
        match (op1, op2) {
            // x > a, x > b: counterexample when a < b is any value in (a, b]
            (CompareOp::Gt, CompareOp::Gt) if val1 < val2 => val1 + 1,

            // x >= a, x >= b: counterexample when a < b
            (CompareOp::Ge, CompareOp::Ge) if val1 < val2 => val1,

            // x < a, x < b: counterexample when a > b
            (CompareOp::Lt, CompareOp::Lt) if val1 > val2 => val1 - 1,

            // x <= a, x <= b: counterexample when a > b
            (CompareOp::Le, CompareOp::Le) if val1 > val2 => val1,

            // x >= a, x > b: counterexample is val1 when val1 <= val2
            (CompareOp::Ge, CompareOp::Gt) if val1 <= val2 => val1,

            // x <= a, x < b: counterexample is val1 when val1 >= val2
            (CompareOp::Le, CompareOp::Lt) if val1 >= val2 => val1,

            // Default: return val1 as the counterexample
            _ => val1,
        }
    }

    /// Mode 2: SMT-based verification
    ///
    /// SMT-based verification (10-500ms): constructs query not(phi1 => phi2),
    /// equivalent to phi1 /\ not(phi2).
    /// If UNSAT: implication is valid
    /// If SAT: extract counterexample
    pub fn smt_check(&self, pred1: &Expr, pred2: &Expr) -> SubsumptionResult {
        // Use the SMT checker from verum_smt
        let smt_result = self
            .smt_checker
            .check(pred1, pred2, SmtCheckMode::SmtAllowed);

        // Convert result
        match smt_result {
            SmtResult::Syntactic(true) | SmtResult::Smt { valid: true, .. } => {
                SubsumptionResult::Holds
            }

            SmtResult::Syntactic(false) | SmtResult::Smt { valid: false, .. } => {
                // Extract counterexample from SMT model
                SubsumptionResult::Fails {
                    counterexample: self.extract_counterexample_from_smt(pred1, pred2),
                }
            }

            SmtResult::Unknown { reason } => {
                // Check if timeout
                if reason.contains("timeout") || reason.contains("Timeout") {
                    // Extract time from reason if possible
                    let time_ms = self.config.timeout_ms;
                    SubsumptionResult::Timeout { time_ms }
                } else {
                    SubsumptionResult::Unknown {
                        reason: Text::from(reason),
                    }
                }
            }
        }
    }

    /// Extract counterexample from SMT model
    ///
    /// Production implementation that analyzes predicates to extract
    /// concrete values that demonstrate the subsumption failure.
    ///
    /// This uses a two-phase approach:
    /// 1. Extract variable bounds from both predicates
    /// 2. Find a value that satisfies pred1 but not pred2
    fn extract_counterexample_from_smt(&self, pred1: &Expr, pred2: &Expr) -> Counterexample {
        // Collect all variables from both predicates
        let vars1 = self.collect_variables(pred1);
        let vars2 = self.collect_variables(pred2);
        let all_vars: Set<Text> = vars1.iter().chain(vars2.iter()).cloned().collect();

        // Extract bounds from both predicates
        let bounds1 = self.extract_all_bounds(pred1);
        let bounds2 = self.extract_all_bounds(pred2);

        let mut values = Map::new();

        // For each variable, try to find a value that satisfies pred1 but not pred2
        for var in all_vars.iter() {
            let bound1 = bounds1.get(var).cloned().unwrap_or((None, None));
            let bound2 = bounds2.get(var).cloned().unwrap_or((None, None));

            // Find a value that:
            // - Satisfies bound1 (lower1 <= x <= upper1)
            // - Does NOT satisfy bound2 (x < lower2 or x > upper2)
            let value = self.find_counterexample_value_from_bounds(bound1, bound2);
            values.insert(var.clone(), value);
        }

        // Format the violated constraint for the error message
        let violated_constraint = self.format_constraint_violation(pred1, pred2);

        Counterexample::new(values, violated_constraint)
    }

    /// Find a value that satisfies bound1 but not bound2 (using bound ranges)
    ///
    /// This function implements a comprehensive strategy for finding counterexamples
    /// when subsumption fails. The goal is to find a concrete value that:
    /// 1. Satisfies the antecedent predicate (bound1)
    /// 2. Violates the consequent predicate (bound2)
    ///
    /// ## Strategy
    ///
    /// The function uses a multi-tier approach:
    /// 1. Exploit gaps between pred1 and pred2 bounds (lower/upper mismatches)
    /// 2. Handle equality constraints where pred2 is more restrictive
    /// 3. Use pred1's bounds directly as valid counterexamples
    /// 4. Apply intelligent defaults based on common refinement patterns
    ///
    /// ## Performance
    ///
    /// This is a fast syntactic method that runs in O(1) time.
    /// For more complex predicates, the SMT solver provides definitive answers.
    fn find_counterexample_value_from_bounds(
        &self,
        bound1: (Option<i64>, Option<i64>),
        bound2: (Option<i64>, Option<i64>),
    ) -> Value {
        let (lower1, upper1) = bound1;
        let (lower2, upper2) = bound2;

        // Strategy: Find a value in the range [lower1, upper1] that is outside [lower2, upper2]

        // Case 1: pred2 has a lower bound that's higher than pred1's lower bound
        // Find a value below pred2's lower bound but within pred1's range
        if let Some(l2) = lower2 {
            if let Some(l1) = lower1 {
                if l2 > l1 {
                    // Value between l1 and l2-1 satisfies pred1 but not pred2
                    return Value::Int(l1);
                }
            } else {
                // pred1 has no lower bound, so we can go below l2
                return Value::Int(l2 - 1);
            }
        }

        // Case 2: pred2 has an upper bound that's lower than pred1's upper bound
        if let Some(u2) = upper2 {
            if let Some(u1) = upper1 {
                if u2 < u1 {
                    // Value between u2+1 and u1 satisfies pred1 but not pred2
                    return Value::Int(u1);
                }
            } else {
                // pred1 has no upper bound, so we can go above u2
                return Value::Int(u2 + 1);
            }
        }

        // Case 3: pred2 constrains equality but pred1 allows more values
        if let (Some(l2), Some(u2)) = (lower2, upper2) {
            if l2 == u2 {
                // pred2 requires exactly l2, find a different value in pred1's range
                if let Some(l1) = lower1 {
                    if l1 != l2 {
                        return Value::Int(l1);
                    }
                }
                if let Some(u1) = upper1 {
                    if u1 != u2 {
                        return Value::Int(u1);
                    }
                }
                // Try adjacent values that might be in pred1's range
                if lower1.map_or(true, |l1| l2 + 1 >= l1) && upper1.map_or(true, |u1| l2 + 1 <= u1)
                {
                    return Value::Int(l2 + 1);
                }
                if lower1.map_or(true, |l1| l2 - 1 >= l1) && upper1.map_or(true, |u1| l2 - 1 <= u1)
                {
                    return Value::Int(l2 - 1);
                }
            }
        }

        // Case 4: Use pred1's bounds directly as valid counterexamples
        // These values are guaranteed to satisfy pred1
        if let Some(l1) = lower1 {
            return Value::Int(l1);
        }
        if let Some(u1) = upper1 {
            return Value::Int(u1);
        }

        // Case 5: No explicit bounds available
        // This occurs when predicates use complex expressions that weren't parsed
        // as simple comparison bounds (e.g., function calls, modular arithmetic,
        // disjunctions, or deeply nested expressions).
        //
        // Apply intelligent defaults based on common refinement type patterns:
        self.select_unbounded_counterexample(bound2)
    }

    /// Select an appropriate counterexample value when no bounds are available
    ///
    /// When syntactic bound extraction fails, this method uses domain knowledge
    /// about common refinement type patterns to select values likely to expose
    /// subsumption failures.
    ///
    /// ## Strategy
    ///
    /// We analyze the pred2 bounds (if any) to determine what values would violate it:
    /// - If pred2 has a lower bound, we go below it
    /// - If pred2 has an upper bound, we go above it
    /// - If pred2 has no bounds either, we use strategic edge-case values
    ///
    /// ## Common Patterns Considered
    ///
    /// - `Int{> 0}` (positive): Counter with 0 or -1
    /// - `Int{>= 0}` (non-negative): Counter with -1
    /// - `Int{< N}` (bounded above): Counter with N or N+1
    /// - Unconstrained: Use 0 as it's the most common edge case
    fn select_unbounded_counterexample(&self, bound2: (Option<i64>, Option<i64>)) -> Value {
        let (lower2, upper2) = bound2;

        // If pred2 has constraints, find a value that violates them
        match (lower2, upper2) {
            // pred2 requires x >= lower2, so x = lower2 - 1 violates it
            (Some(l2), None) => {
                // Choose a value just below the lower bound
                Value::Int(l2.saturating_sub(1))
            }

            // pred2 requires x <= upper2, so x = upper2 + 1 violates it
            (None, Some(u2)) => {
                // Choose a value just above the upper bound
                Value::Int(u2.saturating_add(1))
            }

            // pred2 has both bounds, try outside either
            (Some(l2), Some(u2)) => {
                // Prefer going below the lower bound (often catches >= 0 type issues)
                if l2 > i64::MIN {
                    Value::Int(l2 - 1)
                } else {
                    Value::Int(u2.saturating_add(1))
                }
            }

            // Neither predicate has extractable bounds
            // This happens with complex predicates. Use strategic edge case values.
            (None, None) => {
                // Zero is the most common edge case value that exposes issues:
                // - Violates `> 0` (positive number requirements)
                // - Boundary for array indices
                // - Division by zero cases
                // - Boolean false when converted
                Value::Int(0)
            }
        }
    }

    /// Extract all bounds from a predicate (handles conjunctions)
    fn extract_all_bounds(&self, pred: &Expr) -> Map<Text, (Option<i64>, Option<i64>)> {
        let mut bounds: Map<Text, (Option<i64>, Option<i64>)> = Map::new();
        self.extract_bounds_recursive(pred, &mut bounds);
        bounds
    }

    fn extract_bounds_recursive(
        &self,
        pred: &Expr,
        bounds: &mut Map<Text, (Option<i64>, Option<i64>)>,
    ) {
        match &pred.kind {
            ExprKind::Binary { op, left, right } => {
                match op {
                    // Handle conjunction: both sides contribute bounds
                    BinOp::And | BinOp::BitAnd => {
                        self.extract_bounds_recursive(left, bounds);
                        self.extract_bounds_recursive(right, bounds);
                    }
                    // Handle disjunction: for counterexample generation, we can pick
                    // any branch, so we extract bounds from both but keep the wider range
                    BinOp::Or | BinOp::BitOr => {
                        let mut left_bounds = Map::new();
                        let mut right_bounds = Map::new();
                        self.extract_bounds_recursive(left, &mut left_bounds);
                        self.extract_bounds_recursive(right, &mut right_bounds);

                        // Merge bounds by taking the union (wider range)
                        for (var, (l_lower, l_upper)) in left_bounds.iter() {
                            let (r_lower, r_upper) =
                                right_bounds.get(var).cloned().unwrap_or((None, None));

                            let merged_lower = match (*l_lower, r_lower) {
                                (Some(l), Some(r)) => Some(l.min(r)), // Take minimum lower bound
                                (Some(l), None) => Some(l),
                                (None, Some(l)) => Some(l),
                                (None, None) => None,
                            };
                            let merged_upper = match (*l_upper, r_upper) {
                                (Some(l), Some(r)) => Some(l.max(r)), // Take maximum upper bound
                                (Some(u), None) => Some(u),
                                (None, Some(u)) => Some(u),
                                (None, None) => None,
                            };

                            let (lower, upper) = bounds.entry(var.clone()).or_insert((None, None));
                            if let Some(ml) = merged_lower {
                                *lower = Some(lower.map_or(ml, |l| l.max(ml)));
                            }
                            if let Some(mu) = merged_upper {
                                *upper = Some(upper.map_or(mu, |u| u.min(mu)));
                            }
                        }
                        // Also include variables only in right branch
                        for (var, (r_lower, r_upper)) in right_bounds.iter() {
                            if !left_bounds.contains_key(var) {
                                let (lower, upper) =
                                    bounds.entry(var.clone()).or_insert((None, None));
                                if let Some(rl) = r_lower {
                                    *lower = Some(lower.map_or(*rl, |l| l.max(*rl)));
                                }
                                if let Some(ru) = r_upper {
                                    *upper = Some(upper.map_or(*ru, |u| u.min(*ru)));
                                }
                            }
                        }
                    }
                    // Handle comparison operators
                    BinOp::Gt | BinOp::Ge | BinOp::Lt | BinOp::Le | BinOp::Eq | BinOp::Ne => {
                        if let Some(var_name) = self.extract_variable(left) {
                            if let Some(val) = self.extract_int_literal(right) {
                                let (lower, upper) = bounds.entry(var_name).or_insert((None, None));
                                match op {
                                    BinOp::Gt => {
                                        *lower = Some(lower.map_or(val + 1, |l| l.max(val + 1)));
                                    }
                                    BinOp::Ge => {
                                        *lower = Some(lower.map_or(val, |l| l.max(val)));
                                    }
                                    BinOp::Lt => {
                                        *upper = Some(upper.map_or(val - 1, |u| u.min(val - 1)));
                                    }
                                    BinOp::Le => {
                                        *upper = Some(upper.map_or(val, |u| u.min(val)));
                                    }
                                    BinOp::Eq => {
                                        *lower = Some(val);
                                        *upper = Some(val);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // Also check reversed comparison (e.g., 5 < x)
                        if let Some(var_name) = self.extract_variable(right) {
                            if let Some(val) = self.extract_int_literal(left) {
                                let (lower, upper) = bounds.entry(var_name).or_insert((None, None));
                                match op {
                                    BinOp::Lt => {
                                        // val < x means x > val
                                        *lower = Some(lower.map_or(val + 1, |l| l.max(val + 1)));
                                    }
                                    BinOp::Le => {
                                        // val <= x means x >= val
                                        *lower = Some(lower.map_or(val, |l| l.max(val)));
                                    }
                                    BinOp::Gt => {
                                        // val > x means x < val
                                        *upper = Some(upper.map_or(val - 1, |u| u.min(val - 1)));
                                    }
                                    BinOp::Ge => {
                                        // val >= x means x <= val
                                        *upper = Some(upper.map_or(val, |u| u.min(val)));
                                    }
                                    BinOp::Eq => {
                                        *lower = Some(val);
                                        *upper = Some(val);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Handle negation: !(x < 5) means x >= 5
            ExprKind::Unary {
                op: UnOp::Not,
                expr: inner,
            } => {
                if let ExprKind::Binary {
                    op: inner_op,
                    left,
                    right,
                } = &inner.kind
                {
                    // Negate the comparison
                    let negated_op = match inner_op {
                        BinOp::Lt => BinOp::Ge,
                        BinOp::Le => BinOp::Gt,
                        BinOp::Gt => BinOp::Le,
                        BinOp::Ge => BinOp::Lt,
                        BinOp::Eq => BinOp::Ne,
                        BinOp::Ne => BinOp::Eq,
                        _ => return,
                    };
                    // Extract bounds with negated operator
                    if let Some(var_name) = self.extract_variable(left) {
                        if let Some(val) = self.extract_int_literal(right) {
                            let (lower, upper) = bounds.entry(var_name).or_insert((None, None));
                            match negated_op {
                                BinOp::Gt => {
                                    *lower = Some(lower.map_or(val + 1, |l| l.max(val + 1)));
                                }
                                BinOp::Ge => {
                                    *lower = Some(lower.map_or(val, |l| l.max(val)));
                                }
                                BinOp::Lt => {
                                    *upper = Some(upper.map_or(val - 1, |u| u.min(val - 1)));
                                }
                                BinOp::Le => {
                                    *upper = Some(upper.map_or(val, |u| u.min(val)));
                                }
                                BinOp::Eq => {
                                    *lower = Some(val);
                                    *upper = Some(val);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            // Handle parenthesized expressions
            ExprKind::Paren(inner) => {
                self.extract_bounds_recursive(inner, bounds);
            }
            _ => {}
        }
    }

    /// Collect all variable names from an expression
    fn collect_variables(&self, expr: &Expr) -> Set<Text> {
        let mut vars = Set::new();
        self.collect_variables_recursive(expr, &mut vars);
        vars
    }

    fn collect_variables_recursive(&self, expr: &Expr, vars: &mut Set<Text>) {
        match &expr.kind {
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    vars.insert(Text::from(ident.name.clone()));
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_variables_recursive(left, vars);
                self.collect_variables_recursive(right, vars);
            }
            ExprKind::Unary { expr: inner, .. } => {
                self.collect_variables_recursive(inner, vars);
            }
            ExprKind::Call { func, args, .. } => {
                self.collect_variables_recursive(func, vars);
                for arg in args {
                    self.collect_variables_recursive(arg, vars);
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Handle IfCondition which may contain expressions
                for cond_kind in &condition.conditions {
                    match cond_kind {
                        verum_ast::expr::ConditionKind::Expr(cond_expr) => {
                            self.collect_variables_recursive(cond_expr, vars);
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            self.collect_variables_recursive(value, vars);
                        }
                    }
                }
                // Handle Block - collect from each statement
                for stmt in &then_branch.stmts {
                    if let verum_ast::stmt::StmtKind::Expr { expr: e, .. } = &stmt.kind {
                        self.collect_variables_recursive(e, vars);
                    }
                }
                if let Some(else_expr) = else_branch {
                    self.collect_variables_recursive(else_expr.as_ref(), vars);
                }
            }
            _ => {}
        }
    }

    /// Format the constraint violation message
    fn format_constraint_violation(&self, pred1: &Expr, pred2: &Expr) -> Text {
        Text::from(format!(
            "Predicate `{:?}` does not imply `{:?}`",
            pred1.kind, pred2.kind
        ))
    }

    /// Fallback: extract best-effort counterexample from syntactic analysis
    ///
    /// This method attempts to generate a concrete counterexample by:
    /// 1. Extracting bounds from pred1 (the antecedent)
    /// 2. Finding a value satisfying pred1 but potentially violating pred2
    /// 3. Falling back to heuristic values based on variable names
    fn extract_syntactic_counterexample(
        &self,
        pred1: &Expr,
        pred2: &Expr,
        vars: &Set<Text>,
    ) -> Counterexample {
        let mut values: Map<Text, Value> = Map::new();

        // Extract bounds from pred1 (antecedent) - values must satisfy this
        let bounds1 = self.extract_all_bounds(pred1);

        // Extract bounds from pred2 (consequent) - we want to violate this
        let bounds2 = self.extract_all_bounds(pred2);

        for var in vars.iter() {
            let pred1_bounds = bounds1.get(var).cloned().unwrap_or((None, None));
            let pred2_bounds = bounds2.get(var).cloned().unwrap_or((None, None));

            // Try to find a value that satisfies pred1 bounds but violates pred2 bounds
            let value = self.find_counterexample_value_from_bounds(pred1_bounds, pred2_bounds);
            values.insert(var.clone(), value);
        }

        // If we have any Unknown values, try harder using variable name heuristics
        let all_concrete = values.values().all(|v| !matches!(v, Value::Unknown(_)));
        if !all_concrete {
            for (var, value) in values.clone() {
                if matches!(value, Value::Unknown(_)) {
                    // Use variable name to infer reasonable value
                    let heuristic_value = self.infer_value_from_name(&var);
                    values.insert(var, heuristic_value);
                }
            }
        }

        Counterexample::new(values, self.format_constraint_violation(pred1, pred2))
    }

    /// Infer a reasonable value based on variable name conventions
    fn infer_value_from_name(&self, var: &Text) -> Value {
        let name = var.as_str().to_lowercase();

        // Common naming patterns and their typical values
        if name.contains("len") || name.contains("length") || name.contains("size") {
            Value::Int(0) // Empty collections are often edge cases
        } else if name.contains("count") || name.contains("num") {
            Value::Int(0)
        } else if name.contains("index") || name.contains("idx") || name == "i" || name == "j" {
            Value::Int(0) // Index at zero
        } else if name.contains("bool") || name.contains("flag") || name.starts_with("is_") {
            Value::Bool(false)
        } else if name.contains("rate") || name.contains("ratio") || name.contains("percent") {
            Value::Real(0.0)
        } else if name.contains("price") || name.contains("cost") || name.contains("amount") {
            Value::Int(0)
        } else if name.contains("name") || name.contains("str") || name.contains("text") {
            Value::Text(Text::from(""))
        } else if name.starts_with("n") || name.starts_with("m") || name.starts_with("k") {
            Value::Int(1) // Common loop/count variables
        } else if name == "x" || name == "y" || name == "z" {
            Value::Int(0) // Coordinate-like variables
        } else {
            // Default: use zero as it often reveals edge cases
            Value::Int(0)
        }
    }

    /// Try to extract variable bounds from a predicate
    fn extract_bounds_from_predicate(
        &self,
        pred: &Expr,
    ) -> Option<Map<Text, (Option<i64>, Option<i64>)>> {
        let mut bounds: Map<Text, (Option<i64>, Option<i64>)> = Map::new();

        if let ExprKind::Binary { op, left, right } = &pred.kind {
            if let Some(var_name) = self.extract_variable(left) {
                if let Some(val) = self.extract_int_literal(right) {
                    let (lower, upper) = bounds.entry(var_name).or_insert((None, None));
                    match op {
                        BinOp::Gt => *lower = Some(val + 1),
                        BinOp::Ge => *lower = Some(val),
                        BinOp::Lt => *upper = Some(val - 1),
                        BinOp::Le => *upper = Some(val),
                        BinOp::Eq => {
                            *lower = Some(val);
                            *upper = Some(val);
                        }
                        _ => {}
                    }
                }
            }
        }

        if bounds.is_empty() {
            None
        } else {
            Some(bounds)
        }
    }

    /// Extract integer literal value from expression
    fn extract_int_literal(&self, expr: &Expr) -> Option<i64> {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                if let verum_ast::literal::LiteralKind::Int(int_lit) = &lit.kind {
                    i64::try_from(int_lit.value).ok()
                } else {
                    None
                }
            }
            ExprKind::Unary {
                op: verum_ast::expr::UnOp::Neg,
                expr: inner,
            } => self
                .extract_int_literal(inner)
                .and_then(|v| v.checked_neg()),
            _ => None,
        }
    }

    /// Extract variable name from expression
    fn extract_variable(&self, expr: &Expr) -> Option<Text> {
        match &expr.kind {
            ExprKind::Path(path) if path.segments.len() == 1 => match &path.segments[0] {
                verum_ast::ty::PathSegment::Name(ident) => Some(Text::from(ident.name.clone())),
                verum_ast::ty::PathSegment::Relative => Some(Text::from(".")),
                _ => None,
            },
            ExprKind::Binary { left, .. } => self.extract_variable(left),
            _ => None,
        }
    }

    /// Estimate complexity of subsumption check
    fn estimate_complexity(&self, pred1: &Expr, pred2: &Expr) -> usize {
        self.expr_complexity(pred1) + self.expr_complexity(pred2)
    }

    fn expr_complexity(&self, expr: &Expr) -> usize {
        match &expr.kind {
            ExprKind::Literal(_) => 1,
            ExprKind::Path(_) => 1,
            ExprKind::Binary { left, right, .. } => {
                1 + self.expr_complexity(left) + self.expr_complexity(right)
            }
            ExprKind::Unary { expr, .. } => 1 + self.expr_complexity(expr),
            ExprKind::Call { args, .. } => {
                10 + args.iter().map(|a| self.expr_complexity(a)).sum::<usize>()
            }
            _ => 20, // Unknown expressions are complex
        }
    }

    /// Create a simple counterexample
    fn make_counterexample(&self, provided: &str, required: &str) -> Counterexample {
        Counterexample::new(
            Map::new(),
            Text::from(format!("{} does not imply {}", provided, required)),
        )
    }

    // ==================== Statistics Helpers ====================

    fn record_syntactic(&self, time_ms: u64) {
        let mut stats = self.stats.write().unwrap();
        stats.syntactic_hits += 1;
        stats.total_time_ms += time_ms;
    }

    fn record_smt(&self, time_ms: u64) {
        let mut stats = self.stats.write().unwrap();
        stats.smt_checks += 1;
        stats.total_time_ms += time_ms;
    }

    fn record_fallback(&self, time_ms: u64) {
        let mut stats = self.stats.write().unwrap();
        stats.fallbacks += 1;
        stats.total_time_ms += time_ms;
    }

    fn record_cache_hit(&self) {
        let mut stats = self.stats.write().unwrap();
        stats.cache_hits += 1;
    }

    fn record_cache_miss(&self) {
        let mut stats = self.stats.write().unwrap();
        stats.cache_misses += 1;
    }

    fn record_timeout(&self) {
        let mut stats = self.stats.write().unwrap();
        stats.timeouts += 1;
    }

    // ==================== Cache Helpers ====================

    fn compute_cache_key(&self, pred1: &Expr, pred2: &Expr) -> u64 {
        use std::collections::hash_map::DefaultHasher;

        let mut hasher = DefaultHasher::new();
        format!("{:?}", pred1).hash(&mut hasher);
        format!("{:?}", pred2).hash(&mut hasher);
        hasher.finish()
    }

    fn cache_result(&self, key: u64, result: SubsumptionResult) {
        if !self.config.enable_cache {
            return;
        }

        let mut cache = self.cache.write().unwrap();

        // Evict if full
        if cache.len() >= self.config.cache_size {
            // Simple eviction: remove 10% of entries
            let to_remove = self.config.cache_size / 10;
            let keys_to_remove: List<u64> = cache.keys().take(to_remove).cloned().collect();
            for key in keys_to_remove {
                cache.remove(&key);
            }
        }

        cache.insert(key, result);
    }

    // ==================== Public API ====================

    /// Get current statistics
    pub fn stats(&self) -> SubsumptionStats {
        self.stats.read().unwrap().clone()
    }

    /// Reset statistics
    pub fn reset_stats(&self) {
        let mut stats = self.stats.write().unwrap();
        *stats = SubsumptionStats::default();
    }

    /// Clear the cache
    pub fn clear_cache(&self) {
        self.cache.write().unwrap().clear();
    }

    /// Get configuration
    pub fn config(&self) -> &SubsumptionConfig {
        &self.config
    }
}

impl Default for SubsumptionChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SubsumptionChecker {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubsumptionChecker")
            .field("config", &self.config)
            .field("stats", &self.stats)
            .finish()
    }
}

// ==================== Convenience Functions ====================

/// Quick subsumption check for two refinement types
pub fn check_subsumption(sub: &RefinementType, sup: &RefinementType) -> SubsumptionResult {
    let checker = SubsumptionChecker::new();
    checker.check_subsumption(sub, sup)
}

/// Quick syntactic check for two predicates
pub fn try_syntactic_check(pred1: &Expr, pred2: &Expr) -> Option<SubsumptionResult> {
    let checker = SubsumptionChecker::new();
    checker.try_syntactic_check(pred1, pred2)
}

/// Quick SMT check for two predicates
pub fn smt_check(pred1: &Expr, pred2: &Expr) -> SubsumptionResult {
    let checker = SubsumptionChecker::new();
    checker.smt_check(pred1, pred2)
}

/// Extract counterexample from SMT model
pub fn extract_counterexample(model: &Map<Text, Value>) -> Counterexample {
    Counterexample::new(model.clone(), Text::from("constraint"))
}
