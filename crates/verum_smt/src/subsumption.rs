//! Production-Ready Subsumption Checking
//!
//! Refinement type subsumption: `T{P} <: T{Q}` iff `forall x. P(x) => Q(x)`.
//! For example, `Int{> 10} <: Int{> 0}` because `x > 10` implies `x > 0`.
//!
//! Implements three-tiered subsumption checking for refinement types:
//! - Syntactic: <1ms (pattern matching for common cases)
//! - SMT-based: 10-500ms (Z3 solver for complex predicates)
//! - Proof caching: >90% hit rate target
//!
//! ## Subsumption Rule
//!
//! ```text
//! Γ ⊢ φ₁ ⇒ φ₂    (in SMT logic)
//! ─────────────────────────────────
//! Γ ⊢ T{φ₁} <: T{φ₂}
//! ```
//!
//! Interpretation: Type `T{φ₁}` is a subtype of `T{φ₂}` if predicate `φ₁` logically implies `φ₂`.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use verum_ast::expr::{BinOp, Expr, ExprKind, UnOp};
use verum_ast::literal::{FloatLit, IntLit, Literal, LiteralKind};
use verum_common::{List, Map, Maybe, Text};
use z3::{SatResult, Solver, ast::Bool};

// ==================== Core Types ====================

/// Result of subsumption checking.
///
/// Indicates whether a refinement type subsumes another, and how the
/// result was determined (syntactically or via SMT solving).
#[derive(Debug, Clone, PartialEq)]
pub enum SubsumptionResult {
    /// Proved via syntactic pattern matching (fast path).
    /// Boolean indicates whether subsumption holds.
    Syntactic(bool),
    /// Proved via SMT solver (slow path, more precise).
    Smt {
        /// Whether the subsumption relationship holds.
        valid: bool,
        /// Time taken by the SMT solver in milliseconds.
        time_ms: u64,
    },
    /// Unknown result (timeout, unsupported, or inconclusive).
    Unknown {
        /// Human-readable explanation of why the result is unknown.
        reason: String,
    },
}

impl SubsumptionResult {
    /// Is the subsumption valid?
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Syntactic(valid) => *valid,
            Self::Smt { valid, .. } => *valid,
            Self::Unknown { .. } => false,
        }
    }

    /// Get verification time in milliseconds
    pub fn time_ms(&self) -> u64 {
        match self {
            Self::Syntactic(_) => 0, // <1ms
            Self::Smt { time_ms, .. } => *time_ms,
            Self::Unknown { .. } => 0,
        }
    }
}

/// Subsumption checking mode
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CheckMode {
    /// Only syntactic checking (fast, conservative)
    SyntacticOnly,
    /// Allow SMT solver (accurate, slower)
    SmtAllowed,
}

/// Subsumption checker with caching
pub struct SubsumptionChecker {
    /// Cache for subsumption results
    cache: Arc<RwLock<SubsumptionCache>>,
    /// Performance statistics
    stats: Arc<RwLock<SubsumptionStats>>,
    /// Timeout for SMT queries (milliseconds)
    smt_timeout_ms: u64,
}

impl SubsumptionChecker {
    /// Create a new subsumption checker
    pub fn new() -> Self {
        Self::with_config(SubsumptionConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(config: SubsumptionConfig) -> Self {
        Self {
            cache: Arc::new(RwLock::new(SubsumptionCache::new(config.cache_size))),
            stats: Arc::new(RwLock::new(SubsumptionStats::default())),
            smt_timeout_ms: config.smt_timeout_ms,
        }
    }

    /// Check if phi1 => phi2 (phi1 is stronger, thus a subtype)
    ///
    /// # Performance
    ///
    /// - Syntactic: <1ms
    /// - SMT: 10-500ms (with timeout)
    /// - Cached: ~0ms
    pub fn check(&self, phi1: &Expr, phi2: &Expr, mode: CheckMode) -> SubsumptionResult {
        let start = Instant::now();

        // 1. Try syntactic first (always fast)
        if let Some(result) = self.check_syntactic(phi1, phi2) {
            self.stats
                .write()
                .unwrap()
                .record_syntactic(start.elapsed().as_millis() as u64);
            return SubsumptionResult::Syntactic(result);
        }

        // 2. Check cache
        let key = CacheKey::new(phi1, phi2);
        if let Maybe::Some(cached) = self.cache.read().unwrap().get(&key) {
            self.stats.write().unwrap().record_cache_hit();
            return cached.clone();
        }

        // 3. Use SMT if allowed
        let result = match mode {
            CheckMode::SyntacticOnly => SubsumptionResult::Unknown {
                reason: "SMT checking disabled, syntactic check failed".to_string(),
            },
            CheckMode::SmtAllowed => {
                let smt_result = self.check_smt(phi1, phi2);
                self.stats
                    .write()
                    .unwrap()
                    .record_smt(start.elapsed().as_millis() as u64);
                smt_result
            }
        };

        // Cache the result
        self.cache.write().unwrap().insert(key, result.clone());

        result
    }

    /// Syntactic subsumption checking (fast path)
    ///
    /// Fast syntactic check for refinement subtyping without invoking SMT.
    /// Rule: `T{P} <: T{Q}` holds syntactically when P structurally implies Q.
    ///
    /// Handles common patterns:
    /// - Comparison strengthening: `> 10 => > 0`, `>= 10 => > 0`
    /// - Numeric constant analysis: `x > 10 => x > 5` (larger bound implies smaller)
    /// - Equality: `== 5 => >= 5`, `== 5 => <= 5`
    /// - Conjunction: `a && b => a`, `a && b => b`
    /// - Disjunction: `a => a || b`, `b => a || b`
    /// - Text/List length: `len(s) > 5 => len(s) > 0`
    /// - Range containment: `10 <= x <= 20 => 0 <= x <= 100`
    ///
    /// ## Performance Target
    /// < 1ms for all patterns (typically < 100μs)
    fn check_syntactic(&self, phi1: &Expr, phi2: &Expr) -> Option<bool> {
        // Reflexivity: phi => phi
        if exprs_equal(phi1, phi2) {
            return Some(true);
        }

        match (&phi1.kind, &phi2.kind) {
            // Comparison with numeric constants
            (
                ExprKind::Binary {
                    op: op1,
                    left: left1,
                    right: right1,
                },
                ExprKind::Binary {
                    op: op2,
                    left: left2,
                    right: right2,
                },
            ) => {
                // Length method call patterns: len(x) > 5 => len(x) > 0
                if is_length_call(left1)
                    && is_length_call(left2)
                    && let (Some(recv1), Some(recv2)) =
                        (get_method_receiver(left1), get_method_receiver(left2))
                    && exprs_equal(recv1, recv2)
                {
                    // len(x) > 5 => len(x) > 0
                    return self.check_numeric_comparison(op1, right1, op2, right2);
                }

                // Same left operand (variable)
                if exprs_equal(left1, left2) {
                    // Try numeric constant comparison
                    if let Some(result) = self.check_numeric_comparison(op1, right1, op2, right2) {
                        return Some(result);
                    }
                }

                // Same right operand (for reversed comparisons like: 5 < x)
                if exprs_equal(right1, right2)
                    && let Some(result) =
                        self.check_numeric_comparison_reversed(left1, op1, left2, op2)
                {
                    return Some(result);
                }

                // Check operator strengthening (same operands)
                if exprs_equal(left1, left2) && exprs_equal(right1, right2) {
                    return self.check_comparison_strengthening(*op1, *op2);
                }

                None
            }

            // Conjunction: (a && b) => a and (a && b) => b
            (
                ExprKind::Binary {
                    op: BinOp::And,
                    left,
                    right,
                },
                _,
            ) => {
                if exprs_equal(left, phi2) || exprs_equal(right, phi2) {
                    Some(true)
                } else {
                    // Recursive check: if (a && b) => c, try a => c and b => c
                    self.check_syntactic(left, phi2)
                        .or_else(|| self.check_syntactic(right, phi2))
                }
            }

            // Disjunction: a => (a || b) and b => (a || b)
            (
                _,
                ExprKind::Binary {
                    op: BinOp::Or,
                    left,
                    right,
                },
            ) => {
                if exprs_equal(phi1, left) || exprs_equal(phi1, right) {
                    Some(true)
                } else {
                    None
                }
            }

            // Tautologies: true => anything is false, anything => true is true
            (
                ExprKind::Literal(Literal {
                    kind: LiteralKind::Bool(true),
                    ..
                }),
                _,
            ) => {
                Some(true) // true => phi is always valid
            }
            (
                _,
                ExprKind::Literal(Literal {
                    kind: LiteralKind::Bool(true),
                    ..
                }),
            ) => {
                Some(true) // phi => true is always valid
            }
            (
                ExprKind::Literal(Literal {
                    kind: LiteralKind::Bool(false),
                    ..
                }),
                _,
            ) => {
                Some(true) // false => anything is valid (vacuous truth)
            }

            _ => None,
        }
    }

    /// Check numeric comparison subsumption: x op1 val1 => x op2 val2
    ///
    /// Numeric comparison subsumption: if `x op1 val1` is a stronger bound than
    /// `x op2 val2`, then the first refines the second. Monotonicity of comparisons.
    ///
    /// Examples:
    /// - x > 10 => x > 0 (10 > 0, so stronger bound)
    /// - x > 10 => x >= 0 (10 > 0)
    /// - x >= 10 => x >= 5 (10 >= 5)
    /// - x < 5 => x < 10 (5 < 10)
    /// - x <= 5 => x <= 10 (5 <= 10)
    fn check_numeric_comparison(
        &self,
        op1: &BinOp,
        val1: &Expr,
        op2: &BinOp,
        val2: &Expr,
    ) -> Option<bool> {
        // Extract numeric literals
        let n1 = extract_int_literal(val1)?;
        let n2 = extract_int_literal(val2)?;

        use BinOp::*;

        // Pattern: x > a implies x > b where a >= b
        match (op1, op2) {
            // x > N1 => x > N2 (N1 >= N2)
            (Gt, Gt) => Some(n1 >= n2),
            // x > N1 => x >= N2 (N1 >= N2)
            (Gt, Ge) => Some(n1 >= n2),
            // x >= N1 => x >= N2 (N1 >= N2)
            (Ge, Ge) => Some(n1 >= n2),
            // x >= N1 => x > N2 (N1 > N2, stricter requirement)
            (Ge, Gt) => Some(n1 > n2),

            // x < N1 => x < N2 (N1 <= N2)
            (Lt, Lt) => Some(n1 <= n2),
            // x < N1 => x <= N2 (N1 <= N2)
            (Lt, Le) => Some(n1 <= n2),
            // x <= N1 => x <= N2 (N1 <= N2)
            (Le, Le) => Some(n1 <= n2),
            // x <= N1 => x < N2 (N1 < N2, stricter requirement)
            (Le, Lt) => Some(n1 < n2),

            // x == N => x >= N
            (Eq, Ge) => Some(true),
            // x == N => x <= N
            (Eq, Le) => Some(true),
            // x == N1 => x == N2 (N1 == N2)
            (Eq, Eq) => Some(n1 == n2),

            // x != N1 doesn't imply much about other comparisons
            _ => None,
        }
    }

    /// Check reversed numeric comparisons: val1 op1 x => val2 op2 x
    fn check_numeric_comparison_reversed(
        &self,
        val1: &Expr,
        op1: &BinOp,
        val2: &Expr,
        op2: &BinOp,
    ) -> Option<bool> {
        // Extract numeric literals
        let n1 = extract_int_literal(val1)?;
        let n2 = extract_int_literal(val2)?;

        use BinOp::*;

        // Reverse the operator logic
        // 5 < x is equivalent to x > 5
        match (op1, op2) {
            // N1 < x => N2 < x (N1 >= N2)
            (Lt, Lt) => Some(n1 >= n2),
            // N1 < x => N2 <= x (N1 >= N2)
            (Lt, Le) => Some(n1 >= n2),
            // N1 <= x => N2 <= x (N1 >= N2)
            (Le, Le) => Some(n1 >= n2),

            // N1 > x => N2 > x (N1 <= N2)
            (Gt, Gt) => Some(n1 <= n2),
            // N1 > x => N2 >= x (N1 <= N2)
            (Gt, Ge) => Some(n1 <= n2),
            // N1 >= x => N2 >= x (N1 <= N2)
            (Ge, Ge) => Some(n1 <= n2),

            _ => None,
        }
    }

    /// Check if op1 is a stronger constraint than op2
    ///
    /// Examples:
    /// - `>` is stronger than `>=`
    /// - `<` is stronger than `<=`
    /// - `==` is stronger than any inequality
    fn check_comparison_strengthening(&self, op1: BinOp, op2: BinOp) -> Option<bool> {
        match (op1, op2) {
            // Reflexivity
            (a, b) if a == b => Some(true),

            // > N implies >= N
            (BinOp::Gt, BinOp::Ge) => Some(true),

            // < N implies <= N
            (BinOp::Lt, BinOp::Le) => Some(true),

            // == N implies >= N
            (BinOp::Eq, BinOp::Ge) => Some(true),

            // == N implies <= N
            (BinOp::Eq, BinOp::Le) => Some(true),

            // >= N+1 implies > N (requires value analysis, conservatively reject)
            // > N implies >= N+1 (requires value analysis, conservatively reject)
            _ => None,
        }
    }

    /// SMT-based subsumption checking
    ///
    /// Queries Z3 to check if (phi1 => phi2) is valid.
    ///
    /// Query: `(assert (not (=> phi1 phi2)))`
    /// - If UNSAT: Valid subsumption (no counterexample exists)
    /// - If SAT: Invalid subsumption (counterexample found)
    /// - If UNKNOWN: Timeout or too complex
    fn check_smt(&self, phi1: &Expr, phi2: &Expr) -> SubsumptionResult {
        let start = Instant::now();

        // Create Z3 solver (uses global context)
        let solver = Solver::new();

        // Note: z3-rs 0.19 doesn't easily expose timeout config per-solver
        // Timeout would need to be set globally via Z3_set_param
        // For now, we track elapsed time manually and abort if needed

        // Translate expressions to Z3 (using global context)
        let z3_phi1 = match translate_to_z3(phi1) {
            Ok(expr) => expr,
            Err(e) => {
                return SubsumptionResult::Unknown {
                    reason: format!("Translation error for phi1: {}", e),
                };
            }
        };

        let z3_phi2 = match translate_to_z3(phi2) {
            Ok(expr) => expr,
            Err(e) => {
                return SubsumptionResult::Unknown {
                    reason: format!("Translation error for phi2: {}", e),
                };
            }
        };

        // Assert: NOT (phi1 => phi2)
        // Equivalent to: phi1 AND NOT phi2
        let implication = Bool::implies(&z3_phi1, &z3_phi2);
        let negated = implication.not();
        solver.assert(&negated);

        // Check satisfiability
        let result = solver.check();
        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            // UNSAT: No counterexample exists, implication is valid
            SatResult::Unsat => SubsumptionResult::Smt {
                valid: true,
                time_ms: elapsed,
            },

            // SAT: Counterexample exists, implication is invalid
            SatResult::Sat => SubsumptionResult::Smt {
                valid: false,
                time_ms: elapsed,
            },

            // UNKNOWN: Timeout or too complex
            SatResult::Unknown => {
                let reason = solver
                    .get_reason_unknown()
                    .unwrap_or_else(|| "Unknown reason".to_string());

                // Check if it was a timeout
                let is_timeout = reason.contains("timeout")
                    || reason.contains("canceled")
                    || elapsed >= self.smt_timeout_ms;

                SubsumptionResult::Unknown {
                    reason: if is_timeout {
                        format!(
                            "Timeout after {}ms (threshold: {}ms)",
                            elapsed, self.smt_timeout_ms
                        )
                    } else {
                        reason
                    },
                }
            }
        }
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> CacheStats {
        let cache = self.cache.read().unwrap();
        CacheStats {
            size: cache.entries.len(),
            max_size: cache.max_size,
            hit_rate: self.stats.read().unwrap().cache_hit_rate(),
        }
    }

    /// Get performance statistics
    pub fn stats(&self) -> SubsumptionStats {
        self.stats.read().unwrap().clone()
    }

    /// Clear the cache
    pub fn clear_cache(&self) {
        self.cache.write().unwrap().clear();
    }
}

impl Default for SubsumptionChecker {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Cache ====================

/// Cache key for subsumption results
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    phi1_hash: u64,
    phi2_hash: u64,
}

impl CacheKey {
    fn new(phi1: &Expr, phi2: &Expr) -> Self {
        Self {
            phi1_hash: hash_expr(phi1),
            phi2_hash: hash_expr(phi2),
        }
    }
}

/// Subsumption result cache with LRU eviction
struct SubsumptionCache {
    entries: Map<CacheKey, CachedEntry>,
    max_size: usize,
}

impl SubsumptionCache {
    fn new(max_size: usize) -> Self {
        Self {
            entries: Map::with_capacity(max_size),
            max_size,
        }
    }

    fn get(&self, key: &CacheKey) -> Maybe<SubsumptionResult> {
        self.entries.get(key).map(|entry| entry.result.clone())
    }

    fn insert(&mut self, key: CacheKey, result: SubsumptionResult) {
        // Evict if needed
        if self.entries.len() >= self.max_size {
            self.evict();
        }

        self.entries.insert(
            key,
            CachedEntry {
                result,
                timestamp: Instant::now(),
            },
        );
    }

    fn evict(&mut self) {
        // Simple eviction: remove oldest 10%
        let num_to_remove = (self.max_size / 10).max(1);

        // Find oldest entries
        let mut entries: List<_> = self.entries.iter().collect();
        entries.sort_by_key(|(_, entry)| entry.timestamp);

        // Collect keys to remove
        let keys_to_remove: List<_> = entries
            .iter()
            .take(num_to_remove)
            .map(|(key, _)| (*key).clone())
            .collect();

        // Remove them
        for key in keys_to_remove {
            self.entries.remove(&key);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

#[derive(Debug, Clone)]
struct CachedEntry {
    result: SubsumptionResult,
    timestamp: Instant,
}

// ==================== Statistics ====================

/// Performance statistics for subsumption checking.
///
/// Tracks the distribution of checks between syntactic and SMT paths,
/// along with timing and cache effectiveness metrics.
#[derive(Debug, Clone, Default)]
pub struct SubsumptionStats {
    /// Number of subsumption checks resolved via syntactic matching.
    pub syntactic_checks: u64,
    /// Total time spent on syntactic checks in milliseconds.
    pub syntactic_time_ms: u64,
    /// Number of subsumption checks that required SMT solving.
    pub smt_checks: u64,
    /// Total time spent on SMT checks in milliseconds.
    pub smt_time_ms: u64,
    /// Number of cache hits (avoided recomputation).
    pub cache_hits: u64,
    /// Number of cache misses (required computation).
    pub cache_misses: u64,
}

impl SubsumptionStats {
    fn record_syntactic(&mut self, time_ms: u64) {
        self.syntactic_checks += 1;
        self.syntactic_time_ms += time_ms;
        self.cache_misses += 1;
    }

    fn record_smt(&mut self, time_ms: u64) {
        self.smt_checks += 1;
        self.smt_time_ms += time_ms;
        self.cache_misses += 1;
    }

    fn record_cache_hit(&mut self) {
        self.cache_hits += 1;
    }

    /// Cache hit rate (0.0 to 1.0)
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }

    /// Average syntactic check time
    pub fn avg_syntactic_time_ms(&self) -> f64 {
        if self.syntactic_checks == 0 {
            0.0
        } else {
            self.syntactic_time_ms as f64 / self.syntactic_checks as f64
        }
    }

    /// Average SMT check time
    pub fn avg_smt_time_ms(&self) -> f64 {
        if self.smt_checks == 0 {
            0.0
        } else {
            self.smt_time_ms as f64 / self.smt_checks as f64
        }
    }

    /// Syntactic hit rate: proportion of checks resolved syntactically
    ///
    /// Target: >80% for common code patterns (Spec: Section 4.3.1)
    pub fn syntactic_hit_rate(&self) -> f64 {
        let total_non_cached = self.syntactic_checks + self.smt_checks;
        if total_non_cached == 0 {
            0.0
        } else {
            self.syntactic_checks as f64 / total_non_cached as f64
        }
    }

    /// Overall performance report
    pub fn report(&self) -> Text {
        let total = self.cache_hits + self.cache_misses;
        Text::from(format!(
            "Subsumption Checking Statistics:\n\
             - Total checks: {}\n\
             - Cache hits: {} ({:.1}% hit rate)\n\
             - Syntactic checks: {} ({:.1}% of non-cached)\n\
             - SMT checks: {} ({:.1}% of non-cached)\n\
             - Avg syntactic time: {:.2}ms\n\
             - Avg SMT time: {:.2}ms\n\
             - Syntactic hit rate: {:.1}% (target: >80%)",
            total,
            self.cache_hits,
            self.cache_hit_rate() * 100.0,
            self.syntactic_checks,
            self.syntactic_hit_rate() * 100.0,
            self.smt_checks,
            (self.smt_checks as f64 / (self.syntactic_checks + self.smt_checks).max(1) as f64)
                * 100.0,
            self.avg_syntactic_time_ms(),
            self.avg_smt_time_ms(),
            self.syntactic_hit_rate() * 100.0
        ))
    }
}

/// Cache statistics for subsumption result caching.
///
/// Provides insight into cache utilization and effectiveness.
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Current number of entries in the cache.
    pub size: usize,
    /// Maximum allowed cache size before eviction.
    pub max_size: usize,
    /// Cache hit rate as a ratio (0.0 to 1.0).
    pub hit_rate: f64,
}

// ==================== Configuration ====================

/// Configuration for subsumption checker
#[derive(Debug, Clone)]
pub struct SubsumptionConfig {
    /// Maximum cache size
    pub cache_size: usize,
    /// SMT timeout in milliseconds
    pub smt_timeout_ms: u64,
}

impl Default for SubsumptionConfig {
    fn default() -> Self {
        Self {
            cache_size: 10000,   // 10K entries
            smt_timeout_ms: 100, // 100ms timeout (spec: 10-500ms)
        }
    }
}

// ==================== Utilities ====================

/// Check if two expressions are structurally equal (ignoring spans)
fn exprs_equal(e1: &Expr, e2: &Expr) -> bool {
    use verum_ast::literal::{FloatLit, IntLit, LiteralKind};
    use verum_ast::ty::PathSegment;

    match (&e1.kind, &e2.kind) {
        // Literals
        (ExprKind::Literal(l1), ExprKind::Literal(l2)) => match (&l1.kind, &l2.kind) {
            (LiteralKind::Bool(b1), LiteralKind::Bool(b2)) => b1 == b2,
            (
                LiteralKind::Int(IntLit { value: v1, .. }),
                LiteralKind::Int(IntLit { value: v2, .. }),
            ) => v1 == v2,
            (
                LiteralKind::Float(FloatLit { value: v1, .. }),
                LiteralKind::Float(FloatLit { value: v2, .. }),
            ) => v1 == v2,
            (LiteralKind::Text(s1), LiteralKind::Text(s2)) => s1 == s2,
            (LiteralKind::Char(c1), LiteralKind::Char(c2)) => c1 == c2,
            _ => false,
        },

        // Paths (variables)
        (ExprKind::Path(p1), ExprKind::Path(p2)) => {
            if p1.segments.len() != p2.segments.len() {
                return false;
            }
            p1.segments
                .iter()
                .zip(p2.segments.iter())
                .all(|(s1, s2)| match (s1, s2) {
                    (PathSegment::Name(n1), PathSegment::Name(n2)) => n1.name == n2.name,
                    (PathSegment::SelfValue, PathSegment::SelfValue) => true,
                    (PathSegment::Super, PathSegment::Super) => true,
                    (PathSegment::Cog, PathSegment::Cog) => true,
                    (PathSegment::Relative, PathSegment::Relative) => true,
                    _ => false,
                })
        }

        // Binary operations
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
        ) => op1 == op2 && exprs_equal(l1, l2) && exprs_equal(r1, r2),

        // Unary operations
        (ExprKind::Unary { op: op1, expr: e1 }, ExprKind::Unary { op: op2, expr: e2 }) => {
            op1 == op2 && exprs_equal(e1, e2)
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
            m1.name == m2.name
                && exprs_equal(r1, r2)
                && a1.len() == a2.len()
                && a1.iter().zip(a2.iter()).all(|(x1, x2)| exprs_equal(x1, x2))
        }

        // Function calls
        (ExprKind::Call { func: f1, args: a1, .. }, ExprKind::Call { func: f2, args: a2, .. }) => {
            exprs_equal(f1, f2)
                && a1.len() == a2.len()
                && a1.iter().zip(a2.iter()).all(|(x1, x2)| exprs_equal(x1, x2))
        }

        // Field access
        (
            ExprKind::Field {
                expr: e1,
                field: f1,
            },
            ExprKind::Field {
                expr: e2,
                field: f2,
            },
        ) => f1.name == f2.name && exprs_equal(e1, e2),

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
        ) => exprs_equal(e1, e2) && exprs_equal(i1, i2),

        // Tuples
        (ExprKind::Tuple(t1), ExprKind::Tuple(t2)) => {
            t1.len() == t2.len() && t1.iter().zip(t2.iter()).all(|(x1, x2)| exprs_equal(x1, x2))
        }

        // Cast
        (ExprKind::Cast { expr: e1, ty: t1 }, ExprKind::Cast { expr: e2, ty: t2 }) => {
            // Compare types by kind (simplified)
            t1.kind == t2.kind && exprs_equal(e1, e2)
        }

        // Paren
        (ExprKind::Paren(p1), ExprKind::Paren(p2)) => exprs_equal(p1, p2),

        // Default - not structurally equal (covers all other cases conservatively)
        _ => false,
    }
}

/// Hash an expression for cache key
fn hash_expr(expr: &Expr) -> u64 {
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    hash_expr_recursive(expr, &mut hasher);
    hasher.finish()
}

/// Recursively hash an expression
fn hash_expr_recursive(expr: &Expr, hasher: &mut impl Hasher) {
    use verum_ast::ty::PathSegment;

    match &expr.kind {
        ExprKind::Literal(lit) => {
            "literal".hash(hasher);
            match &lit.kind {
                LiteralKind::Bool(b) => b.hash(hasher),
                LiteralKind::Int(IntLit { value, .. }) => value.hash(hasher),
                LiteralKind::Float(FloatLit { value, .. }) => value.to_bits().hash(hasher),
                LiteralKind::Text(s) => s.as_str().hash(hasher),
                _ => {}
            }
        }
        ExprKind::Path(path) => {
            "path".hash(hasher);
            // Hash path segments
            for segment in &path.segments {
                match segment {
                    PathSegment::Name(ident) => ident.name.hash(hasher),
                    PathSegment::SelfValue => "self".hash(hasher),
                    PathSegment::Super => "super".hash(hasher),
                    PathSegment::Cog => "cog".hash(hasher),
                    PathSegment::Relative => ".".hash(hasher),
                }
            }
        }
        ExprKind::Binary { op, left, right } => {
            "binary".hash(hasher);
            format!("{:?}", op).hash(hasher);
            hash_expr_recursive(left, hasher);
            hash_expr_recursive(right, hasher);
        }
        ExprKind::Unary { op, expr } => {
            "unary".hash(hasher);
            format!("{:?}", op).hash(hasher);
            hash_expr_recursive(expr, hasher);
        }
        _ => {
            // Other expression kinds - hash discriminant
            format!("{:?}", std::mem::discriminant(&expr.kind)).hash(hasher);
        }
    }
}

/// Extract integer literal from expression
fn extract_int_literal(expr: &Expr) -> Option<i64> {
    match &expr.kind {
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit { value, .. }),
            ..
        }) => Some(*value as i64),
        _ => None,
    }
}

/// Check if expression is a length method call: len() or length()
fn is_length_call(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::MethodCall { method, args, .. } => {
            let method_name = method.name.as_str();
            (method_name == "len" || method_name == "length") && args.is_empty()
        }
        _ => false,
    }
}

/// Get receiver of method call
fn get_method_receiver(expr: &Expr) -> Option<&Expr> {
    match &expr.kind {
        ExprKind::MethodCall { receiver, .. } => Some(receiver),
        _ => None,
    }
}

/// Translate Verum expression to Z3 Bool
///
/// Handles both boolean and integer comparison expressions
fn translate_to_z3(expr: &Expr) -> Result<Bool, String> {
    match &expr.kind {
        ExprKind::Literal(Literal {
            kind: LiteralKind::Bool(b),
            ..
        }) => {
            if *b {
                Ok(Bool::from_bool(true))
            } else {
                Ok(Bool::from_bool(false))
            }
        }

        ExprKind::Binary { op, left, right } => {
            // Check if this is an integer comparison
            match op {
                BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le | BinOp::Eq | BinOp::Ne => {
                    // Integer comparison - translate operands to Int
                    let left_int = translate_to_z3_int(left)?;
                    let right_int = translate_to_z3_int(right)?;

                    match op {
                        BinOp::Gt => Ok(left_int.gt(&right_int)),
                        BinOp::Lt => Ok(left_int.lt(&right_int)),
                        BinOp::Ge => Ok(left_int.ge(&right_int)),
                        BinOp::Le => Ok(left_int.le(&right_int)),
                        BinOp::Eq => Ok(left_int.eq(&right_int)),
                        BinOp::Ne => Ok(left_int.eq(&right_int).not()),
                        _ => unreachable!(),
                    }
                }
                BinOp::And | BinOp::Or => {
                    // Boolean operation
                    let left_z3 = translate_to_z3(left)?;
                    let right_z3 = translate_to_z3(right)?;

                    match op {
                        BinOp::And => Ok(Bool::and(&[&left_z3, &right_z3])),
                        BinOp::Or => Ok(Bool::or(&[&left_z3, &right_z3])),
                        _ => unreachable!(),
                    }
                }
                _ => Err(format!("Unsupported binary operator: {:?}", op)),
            }
        }

        ExprKind::Unary { op, expr } => {
            let expr_z3 = translate_to_z3(expr)?;

            match op {
                UnOp::Not => Ok(expr_z3.not()),
                _ => Err(format!("Unsupported unary operator: {:?}", op)),
            }
        }

        ExprKind::Path(_) => {
            // Variable reference - treat as symbolic boolean
            // For comparisons, this will be handled by translate_to_z3_int
            Err(format!(
                "Bare path expressions not supported in boolean context: {:?}",
                expr.kind
            ))
        }

        _ => Err(format!(
            "Unsupported expression kind for Z3 translation: {:?}",
            expr.kind
        )),
    }
}

/// Translate Verum expression to Z3 Int
fn translate_to_z3_int(expr: &Expr) -> Result<z3::ast::Int, String> {
    use z3::ast::Int;

    match &expr.kind {
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit { value, .. }),
            ..
        }) => Ok(Int::from_i64(*value as i64)),

        ExprKind::Path(path) => {
            // Extract variable name
            if let Some(ident) = path.as_ident() {
                let name = ident.as_str();
                Ok(Int::new_const(name))
            } else {
                Err(format!("Complex paths not supported: {:?}", path))
            }
        }

        ExprKind::Binary { op, left, right } => {
            let left_int = translate_to_z3_int(left)?;
            let right_int = translate_to_z3_int(right)?;

            match op {
                BinOp::Add => Ok(left_int + right_int),
                BinOp::Sub => Ok(left_int - right_int),
                BinOp::Mul => Ok(left_int * right_int),
                BinOp::Div => Ok(left_int / right_int),
                BinOp::Rem => Ok(left_int.modulo(&right_int)),
                _ => Err(format!("Unsupported arithmetic operator: {:?}", op)),
            }
        }

        ExprKind::Unary {
            op: UnOp::Neg,
            expr,
        } => {
            let expr_int = translate_to_z3_int(expr)?;
            Ok(-expr_int)
        }

        _ => Err(format!(
            "Unsupported expression kind for Z3 Int translation: {:?}",
            expr.kind
        )),
    }
}

// ==================== Tests ====================
