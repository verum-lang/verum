//! # Capability-Based SMT Solver Router
//!
//! Intelligent dispatcher that routes each proof goal to the most capable
//! SMT solver, using the complementary strengths of Z3 and CVC5.
//!
//! ## Design Philosophy
//!
//! Z3 and CVC5 are both top-tier SMT solvers, but they excel at different
//! theories:
//!
//! | Theory                      | Winner | Rationale                              |
//! |-----------------------------|--------|----------------------------------------|
//! | Linear Integer Arithmetic   | Z3     | Faster LIA decision procedure          |
//! | Bit-vectors                 | Z3     | Aggressive bit-blasting                |
//! | Arrays                      | Z3     | Optimized McCarthy axiom engine        |
//! | E-matching quantifiers      | Z3     | More sophisticated pattern matching    |
//! | Interpolation               | Z3     | CVC5 has no interpolation              |
//! | Optimization (MaxSMT)       | Z3     | Production-grade optimizer             |
//! | Nonlinear Real Arithmetic   | CVC5   | CAD (cylindrical algebraic decomp.)    |
//! | Strings / Regex             | CVC5   | Native string theory                   |
//! | Sequences                   | CVC5   | Z3 has no sequence theory              |
//! | Inductive datatypes         | CVC5   | Coq-inspired datatype reasoning        |
//! | Finite model finding        | CVC5   | Specialized mode                       |
//! | Higher-order logic          | CVC5   | Experimental HOL support               |
//!
//! ## Routing Strategy
//!
//! Given a goal, the router analyzes its structure and chooses one of:
//!
//! 1. **`Z3Only`** — Goal matches a theory where Z3 clearly dominates.
//! 2. **`Cvc5Only`** — Goal matches a theory where CVC5 clearly dominates.
//! 3. **`Portfolio`** — Both solvers run in parallel; first result wins.
//!    Used for mixed-theory goals, large goals, or when no clear winner.
//! 4. **`CrossValidate`** — Both solvers must independently agree. Used for
//!    security-critical verification and proof certificate generation.
//!
//! ## Integration
//!
//! The router is invoked from `BackendSwitcher::solve()` as the first
//! dispatch stage, before any fallback or portfolio logic. It operates on
//! `ProblemCharacteristics` extracted by the `StrategySelector`.
//!
//! ## Example
//!
//! ```rust,ignore
//! use verum_smt::capability_router::{CapabilityRouter, RouterConfig};
//!
//! let router = CapabilityRouter::new(RouterConfig::default());
//! match router.route(&characteristics) {
//!     SolverChoice::Z3Only => /* dispatch to Z3 */,
//!     SolverChoice::Cvc5Only => /* dispatch to CVC5 */,
//!     SolverChoice::Portfolio { timeout_ms } => /* run both in parallel */,
//!     SolverChoice::CrossValidate { strictness } => /* cross-validate */,
//! }
//! ```

use serde::{Deserialize, Serialize};

use crate::strategy_selection::ProblemCharacteristics;

// ============================================================================
// Public types
// ============================================================================

/// Extended problem characteristics including theory-specific flags needed
/// for fine-grained routing decisions.
///
/// This extends the base `ProblemCharacteristics` from `strategy_selection`
/// with additional theory detection not exposed by Z3 probes alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtendedCharacteristics {
    /// Base characteristics from Z3 probes.
    pub base: ProblemCharacteristics,
    /// Goal contains nonlinear real arithmetic (e.g., `x * y * z == 1`).
    pub has_nonlinear_real: bool,
    /// Goal contains nonlinear integer arithmetic.
    pub has_nonlinear_int: bool,
    /// Goal contains string operations (concat, length, contains, regex).
    pub has_strings: bool,
    /// Goal contains sequence operations (seq.extract, seq.at).
    pub has_sequences: bool,
    /// Goal contains regular expression constraints.
    pub has_regex: bool,
    /// Goal contains inductive algebraic datatype reasoning.
    pub has_inductive_datatypes: bool,
    /// Goal uses array theory.
    pub has_arrays: bool,
    /// Goal requires finite model finding.
    pub needs_finite_model_finding: bool,
    /// Goal requires Craig interpolation.
    pub needs_interpolation: bool,
    /// Goal requires optimization (MaxSMT / soft constraints).
    pub needs_optimization: bool,
    /// Maximum bit-width of bit-vectors in the goal (0 if none).
    pub bv_max_width: u32,
    /// Maximum quantifier alternation depth (0 = quantifier-free).
    pub quantifier_depth: u32,
    /// Goal is tagged as security-critical (e.g., `@verify(certified)`).
    pub is_security_critical: bool,
    /// Goal has E-matching patterns declared.
    pub has_patterns: bool,
}

/// The router's decision: which solver(s) to use for a goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SolverChoice {
    /// Use Z3 exclusively.
    Z3Only {
        /// Confidence that Z3 is the right choice (0.0 to 1.0).
        confidence: f64,
        /// Human-readable reason (for diagnostics/logging).
        reason: String,
    },
    /// Use CVC5 exclusively.
    Cvc5Only {
        confidence: f64,
        reason: String,
    },
    /// Run both solvers in parallel; first result wins.
    Portfolio {
        /// Timeout in milliseconds for each solver.
        timeout_ms: u64,
        /// Optional preference when both return the same result simultaneously.
        tie_breaker: TieBreaker,
    },
    /// Both solvers must independently agree on the result.
    /// Divergence indicates a solver bug or encoding issue.
    CrossValidate {
        /// How strict the agreement must be.
        strictness: CrossValidationStrictness,
        /// Timeout per solver (cross-validation runs them sequentially to avoid races).
        timeout_ms: u64,
    },
}

/// Preference when both portfolio solvers return simultaneously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TieBreaker {
    /// Prefer whichever was faster (lower wall-clock time).
    Fastest,
    /// Prefer Z3 (typically more mature proof extraction).
    Z3,
    /// Prefer CVC5 (typically better for complex theories).
    Cvc5,
}

/// How strict cross-validation agreement must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossValidationStrictness {
    /// Only require agreement on SAT/UNSAT result. Models/proofs may differ.
    ResultOnly,
    /// Require matching models when SAT (values must be equal).
    WithModel,
    /// Require both solvers to produce valid (if different) proof objects.
    WithProof,
}

/// Configuration for the capability router.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    /// Enable portfolio mode for hard/mixed goals.
    pub enable_portfolio: bool,
    /// Enable cross-validation for security-critical goals.
    pub enable_cross_validation: bool,
    /// Default timeout for portfolio mode (per solver).
    pub portfolio_timeout_ms: u64,
    /// Default timeout for cross-validation (per solver).
    pub cross_validation_timeout_ms: u64,
    /// Tie-breaker strategy in portfolio mode.
    pub tie_breaker: TieBreaker,
    /// Minimum confidence to route to a single solver (below this → portfolio).
    /// Range: 0.0 (always prefer single solver) to 1.0 (always portfolio unless perfect).
    pub single_solver_confidence_threshold: f64,
    /// Goal complexity threshold above which portfolio mode is forced
    /// (regardless of theory fit).
    pub portfolio_complexity_threshold: f64,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            enable_portfolio: true,
            enable_cross_validation: true,
            portfolio_timeout_ms: 30_000,          // 30s per solver
            cross_validation_timeout_ms: 60_000,   // 60s per solver (cross-val is slower)
            tie_breaker: TieBreaker::Fastest,
            single_solver_confidence_threshold: 0.7,
            portfolio_complexity_threshold: 1000.0,
        }
    }
}

// ============================================================================
// Internal: theory winner tracking
// ============================================================================

/// Internal decision: which solver wins for a particular theory, and how
/// confident we are.
#[derive(Debug, Clone, Copy)]
enum TheoryWinner {
    Z3(f64),
    Cvc5(f64),
}

// ============================================================================
// Router implementation
// ============================================================================

/// The capability-based router: analyzes a goal and chooses the best solver.
#[derive(Debug, Clone)]
pub struct CapabilityRouter {
    config: RouterConfig,
    /// Whether CVC5 is actually available. If false, always routes to Z3.
    cvc5_available: bool,
}

impl CapabilityRouter {
    /// Create a new router with the given configuration.
    ///
    /// Automatically detects CVC5 availability at construction time.
    pub fn new(config: RouterConfig) -> Self {
        Self {
            config,
            cvc5_available: cvc5_sys::init(),
        }
    }

    /// Create a router with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(RouterConfig::default())
    }

    /// Create a router that always uses Z3 (for environments without CVC5).
    pub fn z3_only() -> Self {
        Self {
            config: RouterConfig {
                enable_portfolio: false,
                enable_cross_validation: false,
                ..RouterConfig::default()
            },
            cvc5_available: false,
        }
    }

    /// Override CVC5 availability (for testing).
    #[doc(hidden)]
    pub fn with_cvc5_available(mut self, available: bool) -> Self {
        self.cvc5_available = available;
        self
    }

    /// Return whether CVC5 is available for routing.
    pub fn is_cvc5_available(&self) -> bool {
        self.cvc5_available
    }

    // ------------------------------------------------------------------------
    // Main routing logic
    // ------------------------------------------------------------------------

    /// Main entry point: decide which solver to use for a goal.
    ///
    /// Priority order:
    /// 1. If CVC5 is unavailable → Z3 only
    /// 2. If security-critical → cross-validate
    /// 3. If a theory winner is highly confident → that solver only
    /// 4. If goal is complex/mixed → portfolio
    /// 5. Default → Z3
    pub fn route(&self, chars: &ExtendedCharacteristics) -> SolverChoice {
        // Early exit: no CVC5 means no choice.
        if !self.cvc5_available {
            return SolverChoice::Z3Only {
                confidence: 1.0,
                reason: "CVC5 not available in this build".to_string(),
            };
        }

        // Priority 1: security-critical goals → cross-validate
        if chars.is_security_critical && self.config.enable_cross_validation {
            return SolverChoice::CrossValidate {
                strictness: CrossValidationStrictness::WithProof,
                timeout_ms: self.config.cross_validation_timeout_ms,
            };
        }

        // Priority 2: check for a strong theory winner
        if let Some(winner) = self.theory_winner(chars) {
            match winner {
                TheoryWinner::Z3(confidence)
                    if confidence >= self.config.single_solver_confidence_threshold =>
                {
                    return SolverChoice::Z3Only {
                        confidence,
                        reason: self.z3_reason(chars),
                    };
                }
                TheoryWinner::Cvc5(confidence)
                    if confidence >= self.config.single_solver_confidence_threshold =>
                {
                    return SolverChoice::Cvc5Only {
                        confidence,
                        reason: self.cvc5_reason(chars),
                    };
                }
                _ => {
                    // Weak winner → let portfolio/default handle it
                }
            }
        }

        // Priority 3: complex/mixed goals → portfolio (if enabled)
        if self.config.enable_portfolio
            && (chars.base.size > self.config.portfolio_complexity_threshold
                || self.is_mixed_theory(chars))
        {
            return SolverChoice::Portfolio {
                timeout_ms: self.config.portfolio_timeout_ms,
                tie_breaker: self.config.tie_breaker,
            };
        }

        // Default: Z3 (typically fastest on average)
        SolverChoice::Z3Only {
            confidence: 0.5,
            reason: "no strong theory preference; defaulting to Z3".to_string(),
        }
    }

    // ------------------------------------------------------------------------
    // Theory-specific routing analysis
    // ------------------------------------------------------------------------

    /// Determine if there's a clear theory winner for this goal.
    ///
    /// Returns `Some(winner)` when the goal falls into a theory where one
    /// solver is objectively better. Returns `None` when both solvers are
    /// competitive.
    fn theory_winner(&self, chars: &ExtendedCharacteristics) -> Option<TheoryWinner> {
        // --- Strong CVC5 indicators (0.85+ confidence) ---

        // Sequences: Z3 has no sequence theory — CVC5 wins by default.
        if chars.has_sequences {
            return Some(TheoryWinner::Cvc5(1.0));
        }

        // Strings/regex: CVC5's string theory is state-of-the-art.
        if chars.has_strings || chars.has_regex {
            return Some(TheoryWinner::Cvc5(0.90));
        }

        // Nonlinear real arithmetic: CVC5's CAD is 3-10x faster than Z3.
        if chars.has_nonlinear_real {
            return Some(TheoryWinner::Cvc5(0.95));
        }

        // Finite model finding: CVC5-specific feature.
        if chars.needs_finite_model_finding {
            return Some(TheoryWinner::Cvc5(1.0));
        }

        // Inductive datatypes with deep reasoning: CVC5 has native support.
        if chars.has_inductive_datatypes && chars.quantifier_depth >= 2 {
            return Some(TheoryWinner::Cvc5(0.85));
        }

        // --- Strong Z3 indicators (0.85+ confidence) ---

        // Interpolation: Z3-only feature.
        if chars.needs_interpolation {
            return Some(TheoryWinner::Z3(1.0));
        }

        // Optimization: Z3-only (CVC5's optimization is experimental).
        if chars.needs_optimization {
            return Some(TheoryWinner::Z3(1.0));
        }

        // Large bit-vectors (>32 bits): Z3's bit-blasting is more aggressive.
        if chars.bv_max_width > 32 {
            return Some(TheoryWinner::Z3(0.90));
        }

        // Arrays without nonlinear arithmetic: Z3's array theory is optimized.
        if chars.has_arrays && !chars.has_nonlinear_real && !chars.has_nonlinear_int {
            return Some(TheoryWinner::Z3(0.85));
        }

        // E-matching with explicit patterns: Z3's E-matching is superior.
        if chars.has_patterns && chars.quantifier_depth >= 1 {
            return Some(TheoryWinner::Z3(0.80));
        }

        // --- Medium preferences (0.65-0.80 confidence) ---

        // Pure linear integer arithmetic: Z3 is faster.
        if chars.base.is_qflia && !chars.has_strings && !chars.has_arrays {
            return Some(TheoryWinner::Z3(0.75));
        }

        // Pure propositional: Z3's SAT preprocessing is excellent.
        if chars.base.is_propositional {
            return Some(TheoryWinner::Z3(0.70));
        }

        // Bit-vectors of moderate size: Z3 slightly preferred.
        if chars.base.is_qfbv && chars.bv_max_width <= 32 {
            return Some(TheoryWinner::Z3(0.75));
        }

        // Uninterpreted functions without quantifiers: both competitive, Z3 slightly.
        if chars.base.is_qfuf && chars.quantifier_depth == 0 {
            return Some(TheoryWinner::Z3(0.65));
        }

        // No clear winner.
        None
    }

    /// Detect if the goal mixes multiple theories (making portfolio beneficial).
    fn is_mixed_theory(&self, chars: &ExtendedCharacteristics) -> bool {
        let mut theory_count = 0;
        if chars.base.is_qfbv { theory_count += 1; }
        if chars.base.is_qflia || chars.base.is_qfnra { theory_count += 1; }
        if chars.has_arrays { theory_count += 1; }
        if chars.has_strings { theory_count += 1; }
        if chars.has_inductive_datatypes { theory_count += 1; }
        if chars.base.has_quantifiers { theory_count += 1; }
        theory_count >= 2
    }

    // ------------------------------------------------------------------------
    // Diagnostic reason strings
    // ------------------------------------------------------------------------

    fn z3_reason(&self, chars: &ExtendedCharacteristics) -> String {
        if chars.needs_interpolation {
            "requires Craig interpolation (Z3-only feature)".into()
        } else if chars.needs_optimization {
            "requires MaxSMT optimization (Z3-only feature)".into()
        } else if chars.bv_max_width > 32 {
            format!("large bit-vectors (width={}); Z3 bit-blasting preferred", chars.bv_max_width)
        } else if chars.has_arrays {
            "array theory; Z3's McCarthy engine preferred".into()
        } else if chars.has_patterns {
            "E-matching patterns; Z3's pattern engine preferred".into()
        } else if chars.base.is_qflia {
            "linear integer arithmetic; Z3 faster on LIA".into()
        } else if chars.base.is_propositional {
            "propositional; Z3's SAT preprocessing preferred".into()
        } else if chars.base.is_qfbv {
            "bit-vectors; Z3 slightly preferred".into()
        } else {
            "Z3 is generally competitive".into()
        }
    }

    fn cvc5_reason(&self, chars: &ExtendedCharacteristics) -> String {
        if chars.has_sequences {
            "sequences (Z3 has no sequence theory)".into()
        } else if chars.has_strings || chars.has_regex {
            "strings/regex; CVC5's string theory is state-of-the-art".into()
        } else if chars.has_nonlinear_real {
            "nonlinear real arithmetic; CVC5's CAD is superior".into()
        } else if chars.needs_finite_model_finding {
            "finite model finding (CVC5-specific)".into()
        } else if chars.has_inductive_datatypes {
            "inductive datatypes; CVC5's native support preferred".into()
        } else {
            "CVC5 is generally preferred".into()
        }
    }
}

// ============================================================================
// Characteristic analysis helpers
// ============================================================================

impl ExtendedCharacteristics {
    /// Create empty characteristics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create characteristics from base characteristics plus inferred fields.
    pub fn from_base(base: ProblemCharacteristics) -> Self {
        let has_nonlinear = base.is_qfnra;
        Self {
            base,
            has_nonlinear_real: has_nonlinear,
            ..Default::default()
        }
    }

    /// Compute a complexity score useful for portfolio decisions.
    pub fn complexity_score(&self) -> f64 {
        self.base.size
            + self.base.depth * 10.0
            + self.base.num_consts * 0.5
            + self.base.num_exprs * 0.1
            + (self.quantifier_depth as f64) * 50.0
            + if self.has_nonlinear_real { 100.0 } else { 0.0 }
            + if self.has_strings { 50.0 } else { 0.0 }
    }

    /// Check if the goal is quantifier-free.
    pub fn is_quantifier_free(&self) -> bool {
        self.quantifier_depth == 0 && !self.base.has_quantifiers
    }

    /// Check if the goal involves only theories where both solvers are strong.
    pub fn is_bread_and_butter(&self) -> bool {
        self.is_quantifier_free()
            && !self.has_strings
            && !self.has_sequences
            && !self.has_nonlinear_real
            && !self.has_nonlinear_int
            && !self.has_inductive_datatypes
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn base_chars() -> ProblemCharacteristics {
        ProblemCharacteristics {
            size: 10.0,
            depth: 2.0,
            num_consts: 5.0,
            num_exprs: 20.0,
            is_qfbv: false,
            is_qflia: false,
            is_qfnra: false,
            is_qfuf: false,
            has_quantifiers: false,
            is_propositional: false,
        }
    }

    fn ext_chars(base: ProblemCharacteristics) -> ExtendedCharacteristics {
        ExtendedCharacteristics::from_base(base)
    }

    #[test]
    fn routes_to_z3_when_cvc5_unavailable() {
        let router = CapabilityRouter::z3_only();
        let chars = ext_chars(base_chars());
        match router.route(&chars) {
            SolverChoice::Z3Only { .. } => {}
            other => panic!("expected Z3Only when CVC5 unavailable, got {:?}", other),
        }
    }

    #[test]
    fn routes_strings_to_cvc5() {
        let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.has_strings = true;
        match router.route(&chars) {
            SolverChoice::Cvc5Only { confidence, .. } => {
                assert!(confidence >= 0.85, "strings should have high confidence");
            }
            other => panic!("expected Cvc5Only for strings, got {:?}", other),
        }
    }

    #[test]
    fn routes_nonlinear_real_to_cvc5() {
        let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.has_nonlinear_real = true;
        match router.route(&chars) {
            SolverChoice::Cvc5Only { .. } => {}
            other => panic!("expected Cvc5Only for NRA, got {:?}", other),
        }
    }

    #[test]
    fn routes_sequences_to_cvc5_with_perfect_confidence() {
        let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.has_sequences = true;
        match router.route(&chars) {
            SolverChoice::Cvc5Only { confidence, .. } => {
                assert_eq!(confidence, 1.0, "sequences should have perfect confidence (Z3 has none)");
            }
            other => panic!("expected Cvc5Only with 1.0 confidence, got {:?}", other),
        }
    }

    #[test]
    fn routes_interpolation_to_z3() {
        let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.needs_interpolation = true;
        match router.route(&chars) {
            SolverChoice::Z3Only { confidence, .. } => {
                assert_eq!(confidence, 1.0);
            }
            other => panic!("expected Z3Only for interpolation, got {:?}", other),
        }
    }

    #[test]
    fn routes_large_bitvector_to_z3() {
        let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.base.is_qfbv = true;
        chars.bv_max_width = 64;
        match router.route(&chars) {
            SolverChoice::Z3Only { confidence, .. } => {
                assert!(confidence >= 0.85);
            }
            other => panic!("expected Z3Only for large BV, got {:?}", other),
        }
    }

    #[test]
    fn routes_security_critical_to_cross_validate() {
        let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.is_security_critical = true;
        match router.route(&chars) {
            SolverChoice::CrossValidate { .. } => {}
            other => panic!("expected CrossValidate for security-critical, got {:?}", other),
        }
    }

    #[test]
    fn routes_large_mixed_to_portfolio() {
        // A genuinely mixed goal with no strong theory winner:
        // - Large size (>1000 threshold)
        // - Quantifiers (no patterns, so Z3 E-matching doesn't auto-win)
        // - QFUF (only 0.65 confidence — below 0.7 threshold)
        // - Has arrays + datatypes (mixed theories, no clear winner)
        let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.base.size = 2000.0;
        chars.base.is_qfuf = true;
        chars.base.has_quantifiers = true;
        chars.quantifier_depth = 1;
        // No strong winner: no strings, no NRA, no sequences, no large BV
        // but multiple theories make portfolio beneficial.
        match router.route(&chars) {
            SolverChoice::Portfolio { .. } => {}
            other => panic!("expected Portfolio for large mixed, got {:?}", other),
        }
    }

    #[test]
    fn mixed_theory_detection() {
        let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.base.is_qfbv = true;
        chars.has_arrays = true;
        assert!(router.is_mixed_theory(&chars));
    }

    #[test]
    fn complexity_score_increases_with_size() {
        let mut chars = ext_chars(base_chars());
        let small = chars.complexity_score();
        chars.base.size = 1000.0;
        let large = chars.complexity_score();
        assert!(large > small);
    }

    #[test]
    fn cross_validation_disabled_when_config_says_so() {
        let config = RouterConfig {
            enable_cross_validation: false,
            ..RouterConfig::default()
        };
        let router = CapabilityRouter::new(config).with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.is_security_critical = true;
        match router.route(&chars) {
            SolverChoice::CrossValidate { .. } => {
                panic!("cross-validation should be disabled");
            }
            _ => {} // ok
        }
    }

    #[test]
    fn finite_model_finding_always_cvc5() {
        let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.needs_finite_model_finding = true;
        match router.route(&chars) {
            SolverChoice::Cvc5Only { confidence, .. } => {
                assert_eq!(confidence, 1.0);
            }
            other => panic!("expected Cvc5Only for FMF, got {:?}", other),
        }
    }

    #[test]
    fn bread_and_butter_detection() {
        let mut chars = ext_chars(base_chars());
        chars.base.is_qflia = true;
        assert!(chars.is_bread_and_butter());

        chars.has_strings = true;
        assert!(!chars.is_bread_and_butter());
    }

    #[test]
    fn inductive_datatypes_with_quantifiers_go_to_cvc5() {
        let router = CapabilityRouter::with_defaults().with_cvc5_available(true);
        let mut chars = ext_chars(base_chars());
        chars.has_inductive_datatypes = true;
        chars.quantifier_depth = 2;
        match router.route(&chars) {
            SolverChoice::Cvc5Only { confidence, .. } => {
                assert!(confidence >= 0.85);
            }
            other => panic!("expected Cvc5Only for inductive+quantifiers, got {:?}", other),
        }
    }
}
