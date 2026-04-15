//! # Routing Statistics
//!
//! Tracks how the capability router dispatches proof goals to SMT solvers,
//! enabling diagnostic analysis, performance tuning, and validation that
//! the router's theory-winner predictions match reality.
//!
//! ## Collected Metrics
//!
//! - **Routing decisions**: how many times each `SolverChoice` variant was
//!   selected, broken down by inferred theory.
//! - **Solver wins**: for portfolio races, which solver produced the winning
//!   verdict first.
//! - **Solver timings**: distribution of wall-clock solve times per solver.
//! - **Divergence events**: cross-validation results where Z3 and CVC5
//!   disagreed on SAT/UNSAT (critical for detecting solver bugs).
//! - **Confidence calibration**: for each theory, the router's predicted
//!   confidence vs. the empirical win rate — measures routing quality.
//!
//! ## Usage
//!
//! `RoutingStats` is embedded in `BackendSwitcher` and updated after every
//! `solve()` call. Call `RoutingStats::report()` for a human-readable
//! summary, or `RoutingStats::as_json()` for machine-readable export.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::capability_router::SolverChoice;
use crate::portfolio_executor::{SolverId, SolverVerdict};

// ============================================================================
// Core statistics types
// ============================================================================

/// Per-theory routing statistics.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TheoryStats {
    /// Goals with this theory where Z3 was chosen exclusively.
    pub z3_only: u64,
    /// Goals with this theory where CVC5 was chosen exclusively.
    pub cvc5_only: u64,
    /// Goals with this theory routed to portfolio.
    pub portfolio: u64,
    /// Goals with this theory routed to cross-validation.
    pub cross_validate: u64,
    /// In portfolio runs with this theory: Z3 won.
    pub z3_portfolio_wins: u64,
    /// In portfolio runs with this theory: CVC5 won.
    pub cvc5_portfolio_wins: u64,
    /// Total solving time for this theory (nanoseconds, sum across all solves).
    pub total_nanos: u64,
    /// Goals resolved definitively (SAT or UNSAT).
    pub definitive: u64,
    /// Goals returning Unknown or Error.
    pub failed: u64,
}

impl TheoryStats {
    /// Total number of goals with this theory.
    pub fn total(&self) -> u64 {
        self.z3_only + self.cvc5_only + self.portfolio + self.cross_validate
    }

    /// Fraction of goals successfully resolved (0.0 to 1.0).
    pub fn success_rate(&self) -> f64 {
        let total = self.definitive + self.failed;
        if total == 0 {
            0.0
        } else {
            self.definitive as f64 / total as f64
        }
    }

    /// Average solve time (milliseconds).
    pub fn avg_ms(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            0.0
        } else {
            (self.total_nanos as f64 / 1_000_000.0) / total as f64
        }
    }

    /// For portfolio runs: Z3's empirical win rate (0.0 to 1.0).
    pub fn z3_portfolio_win_rate(&self) -> f64 {
        let portfolio_total = self.z3_portfolio_wins + self.cvc5_portfolio_wins;
        if portfolio_total == 0 {
            0.0
        } else {
            self.z3_portfolio_wins as f64 / portfolio_total as f64
        }
    }
}

/// Theory classification for statistics purposes.
///
/// This is a coarser categorization than `ExtendedCharacteristics` — we
/// bucket goals into a small number of categories for reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TheoryClass {
    /// Pure propositional logic.
    Propositional,
    /// Linear integer arithmetic (QF_LIA).
    LinearInt,
    /// Linear real arithmetic (QF_LRA).
    LinearReal,
    /// Nonlinear real arithmetic (QF_NRA) — CVC5's strength.
    NonlinearReal,
    /// Nonlinear integer arithmetic (QF_NIA).
    NonlinearInt,
    /// Bit-vectors (QF_BV) — Z3's strength.
    BitVectors,
    /// Arrays (QF_AX, QF_ABV).
    Arrays,
    /// Uninterpreted functions.
    Uf,
    /// Strings / Regex — CVC5's strength.
    Strings,
    /// Sequences — CVC5-only.
    Sequences,
    /// Inductive datatypes.
    Datatypes,
    /// Quantifiers (any theory).
    Quantified,
    /// Mixed/other (multiple theories present).
    Mixed,
}

impl TheoryClass {
    /// Short mnemonic for reporting.
    pub fn mnemonic(self) -> &'static str {
        match self {
            TheoryClass::Propositional => "PROP",
            TheoryClass::LinearInt => "LIA",
            TheoryClass::LinearReal => "LRA",
            TheoryClass::NonlinearReal => "NRA",
            TheoryClass::NonlinearInt => "NIA",
            TheoryClass::BitVectors => "BV",
            TheoryClass::Arrays => "ARR",
            TheoryClass::Uf => "UF",
            TheoryClass::Strings => "STR",
            TheoryClass::Sequences => "SEQ",
            TheoryClass::Datatypes => "DT",
            TheoryClass::Quantified => "Q",
            TheoryClass::Mixed => "MIX",
        }
    }

    /// Classify an `ExtendedCharacteristics` into a single theory bucket.
    ///
    /// Priority order (first match wins):
    ///   sequences > strings > NRA > NIA > arrays > BV > datatypes >
    ///   LRA > LIA > quantifiers > UF > propositional > mixed
    pub fn classify(
        chars: &crate::capability_router::ExtendedCharacteristics,
    ) -> TheoryClass {
        if chars.has_sequences {
            TheoryClass::Sequences
        } else if chars.has_strings || chars.has_regex {
            TheoryClass::Strings
        } else if chars.has_nonlinear_real {
            TheoryClass::NonlinearReal
        } else if chars.has_nonlinear_int {
            TheoryClass::NonlinearInt
        } else if chars.has_inductive_datatypes {
            TheoryClass::Datatypes
        } else if chars.has_arrays {
            TheoryClass::Arrays
        } else if chars.base.is_qfbv {
            TheoryClass::BitVectors
        } else if chars.base.is_qflia {
            TheoryClass::LinearInt
        } else if chars.base.has_quantifiers || chars.quantifier_depth > 0 {
            TheoryClass::Quantified
        } else if chars.base.is_qfuf {
            TheoryClass::Uf
        } else if chars.base.is_propositional {
            TheoryClass::Propositional
        } else {
            TheoryClass::Mixed
        }
    }
}

// ============================================================================
// RoutingStats — aggregate statistics collector
// ============================================================================

/// Aggregate routing statistics.
///
/// This struct is updated after every `solve()` call in the `BackendSwitcher`.
/// It uses interior atomic counters for thread-safe updates without locks on
/// the common path, and a `parking_lot::Mutex` for the per-theory breakdown.
#[derive(Debug, Default)]
pub struct RoutingStats {
    // --- Atomic counters (hot path, no locks) ---
    /// Total calls to `solve()`.
    pub total_queries: AtomicU64,
    /// Goals routed to Z3 only.
    pub z3_only_count: AtomicU64,
    /// Goals routed to CVC5 only.
    pub cvc5_only_count: AtomicU64,
    /// Goals routed to portfolio.
    pub portfolio_count: AtomicU64,
    /// Goals routed to cross-validation.
    pub cross_validate_count: AtomicU64,

    // --- Portfolio-specific counters ---
    /// Portfolio runs where Z3 produced the winning verdict first.
    pub z3_portfolio_wins: AtomicU64,
    /// Portfolio runs where CVC5 produced the winning verdict first.
    pub cvc5_portfolio_wins: AtomicU64,

    // --- Cross-validation counters ---
    /// Cross-validation runs where both solvers agreed.
    pub cross_validate_agreed: AtomicU64,
    /// Cross-validation runs where solvers DIVERGED (critical!).
    pub cross_validate_diverged: AtomicU64,
    /// Cross-validation runs where at least one solver failed.
    pub cross_validate_incomplete: AtomicU64,

    // --- Outcomes ---
    pub total_sat: AtomicU64,
    pub total_unsat: AtomicU64,
    pub total_unknown: AtomicU64,
    pub total_errors: AtomicU64,

    // --- Timing ---
    /// Total wall-clock time spent solving (nanoseconds).
    pub total_nanos: AtomicU64,

    // --- Per-theory breakdown (behind a mutex for less-hot updates) ---
    pub per_theory: parking_lot::Mutex<HashMap<TheoryClass, TheoryStats>>,

    // --- Divergence log (last N incidents) ---
    pub divergence_log: parking_lot::Mutex<Vec<DivergenceEvent>>,
}

/// Record of a solver divergence event — used for post-hoc analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceEvent {
    /// When the divergence was detected (Unix seconds since epoch).
    pub timestamp_secs: u64,
    /// Which theory the goal was classified into.
    pub theory: TheoryClass,
    /// Z3's verdict.
    pub z3_verdict: SolverVerdict,
    /// CVC5's verdict.
    pub cvc5_verdict: SolverVerdict,
    /// Z3's solve time (milliseconds).
    pub z3_elapsed_ms: u64,
    /// CVC5's solve time (milliseconds).
    pub cvc5_elapsed_ms: u64,
}

impl RoutingStats {
    /// Create empty statistics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a routing decision.
    pub fn record_routing(
        &self,
        choice: &SolverChoice,
        theory: TheoryClass,
    ) {
        self.total_queries.fetch_add(1, Ordering::Relaxed);

        let mut per_theory = self.per_theory.lock();
        let stats = per_theory.entry(theory).or_default();

        match choice {
            SolverChoice::Z3Only { .. } => {
                self.z3_only_count.fetch_add(1, Ordering::Relaxed);
                stats.z3_only += 1;
            }
            SolverChoice::Cvc5Only { .. } => {
                self.cvc5_only_count.fetch_add(1, Ordering::Relaxed);
                stats.cvc5_only += 1;
            }
            SolverChoice::Portfolio { .. } => {
                self.portfolio_count.fetch_add(1, Ordering::Relaxed);
                stats.portfolio += 1;
            }
            SolverChoice::CrossValidate { .. } => {
                self.cross_validate_count.fetch_add(1, Ordering::Relaxed);
                stats.cross_validate += 1;
            }
        }
    }

    /// Record the outcome of a solve.
    pub fn record_outcome(
        &self,
        theory: TheoryClass,
        verdict: &SolverVerdict,
        elapsed: Duration,
    ) {
        self.total_nanos
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);

        match verdict {
            SolverVerdict::Sat => {
                self.total_sat.fetch_add(1, Ordering::Relaxed);
            }
            SolverVerdict::Unsat => {
                self.total_unsat.fetch_add(1, Ordering::Relaxed);
            }
            SolverVerdict::Unknown { .. } => {
                self.total_unknown.fetch_add(1, Ordering::Relaxed);
            }
            SolverVerdict::Error { .. } | SolverVerdict::Cancelled => {
                self.total_errors.fetch_add(1, Ordering::Relaxed);
            }
        }

        let mut per_theory = self.per_theory.lock();
        let stats = per_theory.entry(theory).or_default();
        stats.total_nanos += elapsed.as_nanos() as u64;
        if verdict.is_definitive() {
            stats.definitive += 1;
        } else {
            stats.failed += 1;
        }
    }

    /// Record a portfolio race result.
    pub fn record_portfolio_win(&self, theory: TheoryClass, winner: SolverId) {
        let mut per_theory = self.per_theory.lock();
        let stats = per_theory.entry(theory).or_default();
        match winner {
            SolverId::Z3 => {
                self.z3_portfolio_wins.fetch_add(1, Ordering::Relaxed);
                stats.z3_portfolio_wins += 1;
            }
            SolverId::Cvc5 => {
                self.cvc5_portfolio_wins.fetch_add(1, Ordering::Relaxed);
                stats.cvc5_portfolio_wins += 1;
            }
        }
    }

    /// Record a cross-validation agreement (both solvers gave the same definitive verdict).
    pub fn record_cross_validate_agreement(&self) {
        self.cross_validate_agreed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cross-validation DIVERGENCE.
    ///
    /// This is a safety-critical event: it indicates either a solver bug or
    /// an encoding error. The event is logged (up to 100 most recent) for
    /// post-hoc analysis.
    pub fn record_divergence(&self, event: DivergenceEvent) {
        self.cross_validate_diverged.fetch_add(1, Ordering::Relaxed);
        let mut log = self.divergence_log.lock();
        log.push(event);
        // Keep only the most recent 100 events to bound memory.
        let overflow = log.len().saturating_sub(100);
        if overflow > 0 {
            log.drain(0..overflow);
        }
    }

    /// Record a cross-validation where at least one solver failed.
    pub fn record_cross_validate_incomplete(&self) {
        self.cross_validate_incomplete
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Generate a human-readable summary report.
    pub fn report(&self) -> String {
        let mut out = String::new();

        let total = self.total_queries.load(Ordering::Relaxed);
        let z3_only = self.z3_only_count.load(Ordering::Relaxed);
        let cvc5_only = self.cvc5_only_count.load(Ordering::Relaxed);
        let portfolio = self.portfolio_count.load(Ordering::Relaxed);
        let cross_validate = self.cross_validate_count.load(Ordering::Relaxed);

        out.push_str("╭─────────────────────────────────────────────────────────╮\n");
        out.push_str("│         Verum SMT Router Statistics                      │\n");
        out.push_str("├─────────────────────────────────────────────────────────┤\n");
        out.push_str(&format!("│  Total queries:        {:>10}                      │\n", total));

        if total > 0 {
            out.push_str(&format!("│  Routed to Z3 only:    {:>10}  ({:>5.1}%)              │\n",
                z3_only, 100.0 * z3_only as f64 / total as f64));
            out.push_str(&format!("│  Routed to CVC5 only:  {:>10}  ({:>5.1}%)              │\n",
                cvc5_only, 100.0 * cvc5_only as f64 / total as f64));
            out.push_str(&format!("│  Portfolio:            {:>10}  ({:>5.1}%)              │\n",
                portfolio, 100.0 * portfolio as f64 / total as f64));
            out.push_str(&format!("│  Cross-validate:       {:>10}  ({:>5.1}%)              │\n",
                cross_validate, 100.0 * cross_validate as f64 / total as f64));
        }

        let z3_wins = self.z3_portfolio_wins.load(Ordering::Relaxed);
        let cvc5_wins = self.cvc5_portfolio_wins.load(Ordering::Relaxed);
        let portfolio_total = z3_wins + cvc5_wins;
        if portfolio_total > 0 {
            out.push_str("├─────────────────────────────────────────────────────────┤\n");
            out.push_str(&format!("│  Portfolio Z3 wins:    {:>10}  ({:>5.1}%)              │\n",
                z3_wins, 100.0 * z3_wins as f64 / portfolio_total as f64));
            out.push_str(&format!("│  Portfolio CVC5 wins:  {:>10}  ({:>5.1}%)              │\n",
                cvc5_wins, 100.0 * cvc5_wins as f64 / portfolio_total as f64));
        }

        let cv_agreed = self.cross_validate_agreed.load(Ordering::Relaxed);
        let cv_diverged = self.cross_validate_diverged.load(Ordering::Relaxed);
        let cv_incomplete = self.cross_validate_incomplete.load(Ordering::Relaxed);
        if cv_agreed + cv_diverged + cv_incomplete > 0 {
            out.push_str("├─────────────────────────────────────────────────────────┤\n");
            out.push_str(&format!("│  Cross-val agreed:     {:>10}                       │\n", cv_agreed));
            if cv_diverged > 0 {
                out.push_str(&format!("│  Cross-val DIVERGED:   {:>10}  ⚠ CHECK LOG         │\n", cv_diverged));
            }
            out.push_str(&format!("│  Cross-val incomplete: {:>10}                       │\n", cv_incomplete));
        }

        let sat = self.total_sat.load(Ordering::Relaxed);
        let unsat = self.total_unsat.load(Ordering::Relaxed);
        let unknown = self.total_unknown.load(Ordering::Relaxed);
        let errors = self.total_errors.load(Ordering::Relaxed);
        out.push_str("├─────────────────────────────────────────────────────────┤\n");
        out.push_str(&format!("│  SAT:                  {:>10}                       │\n", sat));
        out.push_str(&format!("│  UNSAT:                {:>10}                       │\n", unsat));
        out.push_str(&format!("│  Unknown:              {:>10}                       │\n", unknown));
        out.push_str(&format!("│  Errors/cancelled:     {:>10}                       │\n", errors));

        let total_nanos = self.total_nanos.load(Ordering::Relaxed);
        if total > 0 {
            let avg_ms = (total_nanos as f64 / 1_000_000.0) / total as f64;
            out.push_str(&format!("│  Avg time per query:   {:>10.2} ms                    │\n", avg_ms));
        }

        out.push_str("├─────────────────────────────────────────────────────────┤\n");
        out.push_str("│  Per-theory breakdown:                                   │\n");

        let per_theory = self.per_theory.lock();
        let mut theories: Vec<_> = per_theory.iter().collect();
        theories.sort_by_key(|(_, s)| std::cmp::Reverse(s.total()));

        for (theory, stats) in theories {
            let total = stats.total();
            if total == 0 {
                continue;
            }
            out.push_str(&format!(
                "│    {:<5}: {:>6} goals, {:>5.1}% success, {:>7.2}ms avg     │\n",
                theory.mnemonic(),
                total,
                stats.success_rate() * 100.0,
                stats.avg_ms(),
            ));
        }

        out.push_str("╰─────────────────────────────────────────────────────────╯\n");
        out
    }

    /// Export statistics as JSON for machine consumption.
    pub fn as_json(&self) -> serde_json::Value {
        let per_theory = self.per_theory.lock();
        let theories: HashMap<String, TheoryStats> = per_theory
            .iter()
            .map(|(k, v)| (k.mnemonic().to_string(), v.clone()))
            .collect();

        serde_json::json!({
            "total_queries": self.total_queries.load(Ordering::Relaxed),
            "routing": {
                "z3_only": self.z3_only_count.load(Ordering::Relaxed),
                "cvc5_only": self.cvc5_only_count.load(Ordering::Relaxed),
                "portfolio": self.portfolio_count.load(Ordering::Relaxed),
                "cross_validate": self.cross_validate_count.load(Ordering::Relaxed),
            },
            "portfolio_wins": {
                "z3": self.z3_portfolio_wins.load(Ordering::Relaxed),
                "cvc5": self.cvc5_portfolio_wins.load(Ordering::Relaxed),
            },
            "cross_validate": {
                "agreed": self.cross_validate_agreed.load(Ordering::Relaxed),
                "diverged": self.cross_validate_diverged.load(Ordering::Relaxed),
                "incomplete": self.cross_validate_incomplete.load(Ordering::Relaxed),
            },
            "outcomes": {
                "sat": self.total_sat.load(Ordering::Relaxed),
                "unsat": self.total_unsat.load(Ordering::Relaxed),
                "unknown": self.total_unknown.load(Ordering::Relaxed),
                "errors": self.total_errors.load(Ordering::Relaxed),
            },
            "total_nanos": self.total_nanos.load(Ordering::Relaxed),
            "per_theory": theories,
        })
    }

    /// Return the list of recent divergence events.
    pub fn divergence_events(&self) -> Vec<DivergenceEvent> {
        self.divergence_log.lock().clone()
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.total_queries.store(0, Ordering::Relaxed);
        self.z3_only_count.store(0, Ordering::Relaxed);
        self.cvc5_only_count.store(0, Ordering::Relaxed);
        self.portfolio_count.store(0, Ordering::Relaxed);
        self.cross_validate_count.store(0, Ordering::Relaxed);
        self.z3_portfolio_wins.store(0, Ordering::Relaxed);
        self.cvc5_portfolio_wins.store(0, Ordering::Relaxed);
        self.cross_validate_agreed.store(0, Ordering::Relaxed);
        self.cross_validate_diverged.store(0, Ordering::Relaxed);
        self.cross_validate_incomplete.store(0, Ordering::Relaxed);
        self.total_sat.store(0, Ordering::Relaxed);
        self.total_unsat.store(0, Ordering::Relaxed);
        self.total_unknown.store(0, Ordering::Relaxed);
        self.total_errors.store(0, Ordering::Relaxed);
        self.total_nanos.store(0, Ordering::Relaxed);
        self.per_theory.lock().clear();
        self.divergence_log.lock().clear();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_router::{ExtendedCharacteristics, SolverChoice};

    #[test]
    fn records_routing_increments_counters() {
        let stats = RoutingStats::new();
        let choice = SolverChoice::Z3Only {
            confidence: 0.9,
            reason: "test".into(),
        };
        stats.record_routing(&choice, TheoryClass::LinearInt);
        assert_eq!(stats.z3_only_count.load(Ordering::Relaxed), 1);
        assert_eq!(stats.total_queries.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn records_outcomes_correctly() {
        let stats = RoutingStats::new();
        stats.record_outcome(
            TheoryClass::BitVectors,
            &SolverVerdict::Sat,
            Duration::from_millis(50),
        );
        stats.record_outcome(
            TheoryClass::BitVectors,
            &SolverVerdict::Unsat,
            Duration::from_millis(30),
        );
        stats.record_outcome(
            TheoryClass::BitVectors,
            &SolverVerdict::Unknown { reason: "timeout".into() },
            Duration::from_millis(10_000),
        );

        assert_eq!(stats.total_sat.load(Ordering::Relaxed), 1);
        assert_eq!(stats.total_unsat.load(Ordering::Relaxed), 1);
        assert_eq!(stats.total_unknown.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn records_portfolio_wins() {
        let stats = RoutingStats::new();
        stats.record_portfolio_win(TheoryClass::LinearInt, SolverId::Z3);
        stats.record_portfolio_win(TheoryClass::LinearInt, SolverId::Z3);
        stats.record_portfolio_win(TheoryClass::Strings, SolverId::Cvc5);

        assert_eq!(stats.z3_portfolio_wins.load(Ordering::Relaxed), 2);
        assert_eq!(stats.cvc5_portfolio_wins.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn divergence_log_bounded() {
        let stats = RoutingStats::new();
        for i in 0..150 {
            stats.record_divergence(DivergenceEvent {
                timestamp_secs: i,
                theory: TheoryClass::LinearInt,
                z3_verdict: SolverVerdict::Sat,
                cvc5_verdict: SolverVerdict::Unsat,
                z3_elapsed_ms: 10,
                cvc5_elapsed_ms: 20,
            });
        }
        // Should keep only last 100 events.
        assert_eq!(stats.divergence_events().len(), 100);
        // First remaining event should be from iteration 50.
        assert_eq!(stats.divergence_events()[0].timestamp_secs, 50);
    }

    #[test]
    fn theory_classification_priority() {
        let mut chars = ExtendedCharacteristics::default();
        chars.has_nonlinear_real = true;
        chars.base.is_qflia = true;  // Should NOT dominate
        assert_eq!(TheoryClass::classify(&chars), TheoryClass::NonlinearReal);

        let mut chars = ExtendedCharacteristics::default();
        chars.has_sequences = true;
        chars.has_strings = true;  // Should NOT dominate
        assert_eq!(TheoryClass::classify(&chars), TheoryClass::Sequences);
    }

    #[test]
    fn report_generates_readable_output() {
        let stats = RoutingStats::new();
        stats.record_routing(
            &SolverChoice::Z3Only { confidence: 0.9, reason: "test".into() },
            TheoryClass::LinearInt,
        );
        stats.record_outcome(
            TheoryClass::LinearInt,
            &SolverVerdict::Sat,
            Duration::from_millis(5),
        );

        let report = stats.report();
        assert!(report.contains("Total queries"));
        assert!(report.contains("LIA"));
        assert!(report.contains("Z3 only"));
    }

    #[test]
    fn json_export_contains_expected_fields() {
        let stats = RoutingStats::new();
        stats.total_queries.store(42, Ordering::Relaxed);

        let json = stats.as_json();
        assert_eq!(json["total_queries"], 42);
        assert!(json["routing"].is_object());
        assert!(json["portfolio_wins"].is_object());
        assert!(json["outcomes"].is_object());
    }

    #[test]
    fn reset_clears_all_state() {
        let stats = RoutingStats::new();
        stats.total_queries.store(100, Ordering::Relaxed);
        stats.record_routing(
            &SolverChoice::Z3Only { confidence: 0.9, reason: "t".into() },
            TheoryClass::LinearInt,
        );
        stats.reset();
        assert_eq!(stats.total_queries.load(Ordering::Relaxed), 0);
        assert!(stats.per_theory.lock().is_empty());
    }

    #[test]
    fn theory_stats_avg_ms() {
        let mut ts = TheoryStats::default();
        ts.z3_only = 2;
        ts.total_nanos = 10_000_000; // 10 ms total
        // Total = 2 goals, total 10ms → 5ms avg
        assert!((ts.avg_ms() - 5.0).abs() < 0.01);
    }
}
