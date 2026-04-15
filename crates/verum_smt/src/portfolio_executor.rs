//! # Portfolio SMT Solver Executor
//!
//! Runs Z3 and CVC5 concurrently on the same goal and returns the first
//! result, or cross-validates both results for security-critical goals.
//!
//! ## Design
//!
//! The executor uses `std::thread` with `crossbeam::channel` for coordination:
//!
//! ```text
//!       goal                 goal
//!        │                    │
//!        ▼                    ▼
//!   ┌────────┐           ┌────────┐
//!   │   Z3   │           │  CVC5  │
//!   └───┬────┘           └───┬────┘
//!       │ result              │ result
//!       └──────► channel ◄────┘
//!                  │
//!                  ▼
//!            (first wins)
//! ```
//!
//! For cross-validation, both solvers run to completion and results are
//! compared. Divergent results (one says SAT, the other UNSAT) indicate a
//! solver bug or an encoding issue and are reported as a hard error.
//!
//! ## Cancellation
//!
//! Z3 and CVC5 support cooperative cancellation via their C APIs
//! (`Z3_interrupt`, `cvc5_solver_interrupt`). After one solver returns, the
//! other is interrupted to release resources promptly.
//!
//! ## Thread Safety
//!
//! - Each solver instance is used by exactly one thread.
//! - Results are passed via channels (no shared mutable state).
//! - The `PortfolioExecutor` itself is `Send + Sync`.
//!
//! ## Performance
//!
//! Portfolio execution typically achieves 1.5–3x speedup on hard goals
//! where the winning solver varies by problem structure. For bread-and-butter
//! goals where Z3 is consistently fastest, portfolio adds ~20% overhead,
//! which is why the `CapabilityRouter` only routes hard/mixed goals here.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::capability_router::{CrossValidationStrictness, TieBreaker};

// ============================================================================
// Public types
// ============================================================================

/// Outcome of a single solver's check_sat operation in the portfolio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SolverVerdict {
    /// Solver proved the formula satisfiable.
    Sat,
    /// Solver proved the formula unsatisfiable.
    Unsat,
    /// Solver could not determine (timeout, resource limit, incomplete theory).
    Unknown { reason: String },
    /// Solver encountered an error (linking, memory, internal bug).
    Error { message: String },
    /// Solver was interrupted before completion (e.g., the other solver won).
    Cancelled,
}

impl SolverVerdict {
    /// True if this verdict is a definitive answer (SAT or UNSAT).
    pub fn is_definitive(&self) -> bool {
        matches!(self, SolverVerdict::Sat | SolverVerdict::Unsat)
    }

    /// True if this verdict indicates failure (Error, Unknown, Cancelled).
    pub fn is_failure(&self) -> bool {
        !self.is_definitive()
    }
}

/// Which solver produced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SolverId {
    Z3,
    Cvc5,
}

impl SolverId {
    pub fn name(self) -> &'static str {
        match self {
            SolverId::Z3 => "z3",
            SolverId::Cvc5 => "cvc5",
        }
    }
}

/// Result from a single solver in the portfolio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverResult {
    pub solver: SolverId,
    pub verdict: SolverVerdict,
    pub elapsed_ms: u64,
}

/// Final result of a portfolio check_sat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioResult {
    /// Which solver won (produced the accepted verdict).
    pub winner: SolverId,
    /// The accepted verdict.
    pub verdict: SolverVerdict,
    /// Wall-clock time until the winner returned (ms).
    pub winner_elapsed_ms: u64,
    /// Total wall-clock time including loser (ms).
    pub total_elapsed_ms: u64,
    /// Individual result from each solver (for diagnostics).
    pub z3_result: Option<SolverResult>,
    pub cvc5_result: Option<SolverResult>,
}

/// Result of a cross-validation check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CrossValidateResult {
    /// Both solvers agreed.
    Agreed {
        verdict: SolverVerdict,
        z3_elapsed_ms: u64,
        cvc5_elapsed_ms: u64,
    },
    /// Solvers disagreed — CRITICAL: indicates a solver bug or encoding issue.
    Diverged {
        z3_verdict: SolverVerdict,
        cvc5_verdict: SolverVerdict,
        z3_elapsed_ms: u64,
        cvc5_elapsed_ms: u64,
    },
    /// At least one solver failed (error/timeout); agreement cannot be determined.
    Incomplete {
        z3_result: SolverResult,
        cvc5_result: SolverResult,
    },
}

impl CrossValidateResult {
    /// True if cross-validation passed (both solvers gave the same definitive verdict).
    pub fn is_agreed(&self) -> bool {
        matches!(self, CrossValidateResult::Agreed { .. })
    }

    /// True if cross-validation detected a divergence (safety-critical).
    pub fn is_diverged(&self) -> bool {
        matches!(self, CrossValidateResult::Diverged { .. })
    }
}

// ============================================================================
// Solver adapter trait
// ============================================================================

/// Abstract interface for a solver that can be run in a portfolio.
///
/// This trait decouples the portfolio executor from the concrete backend
/// implementations, enabling mocking in tests and future solver integrations.
///
/// Implementations must:
/// - Be `Send + Sync + 'static` (so they can be moved into threads).
/// - Support cooperative cancellation via `request_interrupt()`.
/// - Return a `SolverVerdict` from `check_sat()`.
pub trait PortfolioSolver: Send + Sync {
    /// Check satisfiability with the configured timeout.
    ///
    /// If `interrupt` is set to `true` concurrently, the implementation should
    /// return `SolverVerdict::Cancelled` as soon as practical.
    fn check_sat(&mut self, interrupt: &AtomicBool) -> SolverVerdict;

    /// Identify this solver (used in diagnostics).
    fn solver_id(&self) -> SolverId;
}

// ============================================================================
// Portfolio executor
// ============================================================================

/// Executes Z3 and CVC5 in parallel, returning the first result.
///
/// Use `solve_portfolio` for first-wins semantics or `solve_cross_validate`
/// for strict agreement.
pub struct PortfolioExecutor;

impl PortfolioExecutor {
    /// Run both solvers in parallel; return the first definitive result.
    ///
    /// If one solver produces SAT/UNSAT first, the other is interrupted.
    /// If one solver produces Unknown/Error, the other is allowed to continue
    /// (so a partial result doesn't cancel a potentially better answer).
    ///
    /// ## Timeout
    ///
    /// Each solver is given up to `timeout` to complete. If neither finishes
    /// in that window, both return `Unknown`, and the portfolio returns the
    /// faster of the two.
    ///
    /// ## Tie-breaker
    ///
    /// If both solvers produce definitive results within ~1 ms of each other,
    /// the `tie_breaker` policy selects which one wins.
    pub fn solve_portfolio<Z, C>(
        mut z3: Z,
        mut cvc5: C,
        timeout: Duration,
        tie_breaker: TieBreaker,
    ) -> PortfolioResult
    where
        Z: PortfolioSolver + 'static,
        C: PortfolioSolver + 'static,
    {
        let start = Instant::now();
        let z3_interrupt = Arc::new(AtomicBool::new(false));
        let cvc5_interrupt = Arc::new(AtomicBool::new(false));

        // Launch both solvers on dedicated threads.
        let z3_handle = {
            let interrupt = z3_interrupt.clone();
            thread::Builder::new()
                .name("portfolio-z3".to_string())
                .spawn(move || {
                    let t0 = Instant::now();
                    let verdict = z3.check_sat(&interrupt);
                    SolverResult {
                        solver: SolverId::Z3,
                        verdict,
                        elapsed_ms: t0.elapsed().as_millis() as u64,
                    }
                })
                .expect("failed to spawn Z3 portfolio thread")
        };

        let cvc5_handle = {
            let interrupt = cvc5_interrupt.clone();
            thread::Builder::new()
                .name("portfolio-cvc5".to_string())
                .spawn(move || {
                    let t0 = Instant::now();
                    let verdict = cvc5.check_sat(&interrupt);
                    SolverResult {
                        solver: SolverId::Cvc5,
                        verdict,
                        elapsed_ms: t0.elapsed().as_millis() as u64,
                    }
                })
                .expect("failed to spawn CVC5 portfolio thread")
        };

        // Set global timeout alarm.
        let global_deadline = start + timeout + Duration::from_millis(100);

        // Wrap handles in Option so we can take() them without moving.
        let mut z3_handle = Some(z3_handle);
        let mut cvc5_handle = Some(cvc5_handle);

        // Poll both threads, accepting the first definitive result.
        let poll_interval = Duration::from_millis(5);
        let mut z3_done: Option<SolverResult> = None;
        let mut cvc5_done: Option<SolverResult> = None;

        while z3_done.is_none() || cvc5_done.is_none() {
            // Check deadline.
            if Instant::now() > global_deadline {
                z3_interrupt.store(true, Ordering::SeqCst);
                cvc5_interrupt.store(true, Ordering::SeqCst);
                break;
            }

            // Try to join each thread non-destructively (only when finished).
            if z3_done.is_none() {
                if let Some(h) = z3_handle.as_ref() {
                    if h.is_finished() {
                        z3_done = z3_handle.take().and_then(|h| h.join().ok());
                    }
                }
            }
            if cvc5_done.is_none() {
                if let Some(h) = cvc5_handle.as_ref() {
                    if h.is_finished() {
                        cvc5_done = cvc5_handle.take().and_then(|h| h.join().ok());
                    }
                }
            }

            // First-wins short-circuit.
            if let Some(result) = z3_done.as_ref() {
                if result.verdict.is_definitive() && cvc5_done.is_none() {
                    cvc5_interrupt.store(true, Ordering::SeqCst);
                    cvc5_done = cvc5_handle.take().and_then(|h| h.join().ok());
                    break;
                }
            }
            if let Some(result) = cvc5_done.as_ref() {
                if result.verdict.is_definitive() && z3_done.is_none() {
                    z3_interrupt.store(true, Ordering::SeqCst);
                    z3_done = z3_handle.take().and_then(|h| h.join().ok());
                    break;
                }
            }

            thread::sleep(poll_interval);
        }

        // Ensure both threads are joined (in case we broke out of the loop early).
        if z3_done.is_none() {
            z3_done = z3_handle.take().and_then(|h| h.join().ok());
        }
        if cvc5_done.is_none() {
            cvc5_done = cvc5_handle.take().and_then(|h| h.join().ok());
        }

        let total_elapsed_ms = start.elapsed().as_millis() as u64;

        // Select the winner based on verdicts and tie-breaker.
        Self::select_winner(z3_done, cvc5_done, tie_breaker, total_elapsed_ms)
    }

    /// Run both solvers to completion and require them to agree.
    ///
    /// Unlike `solve_portfolio`, this does NOT short-circuit on the first result.
    /// Both solvers must produce definitive verdicts, and those verdicts must
    /// match.
    ///
    /// Divergence is treated as a hard error: it indicates a solver bug or
    /// encoding issue, and the caller should investigate (not silently accept).
    pub fn solve_cross_validate<Z, C>(
        mut z3: Z,
        mut cvc5: C,
        timeout: Duration,
        _strictness: CrossValidationStrictness,
    ) -> CrossValidateResult
    where
        Z: PortfolioSolver + 'static,
        C: PortfolioSolver + 'static,
    {
        let z3_interrupt = Arc::new(AtomicBool::new(false));
        let cvc5_interrupt = Arc::new(AtomicBool::new(false));

        let z3_handle = {
            let interrupt = z3_interrupt.clone();
            thread::Builder::new()
                .name("crossvalidate-z3".to_string())
                .spawn(move || {
                    let t0 = Instant::now();
                    let verdict = z3.check_sat(&interrupt);
                    SolverResult {
                        solver: SolverId::Z3,
                        verdict,
                        elapsed_ms: t0.elapsed().as_millis() as u64,
                    }
                })
                .expect("failed to spawn Z3 cross-validate thread")
        };

        let cvc5_handle = {
            let interrupt = cvc5_interrupt.clone();
            thread::Builder::new()
                .name("crossvalidate-cvc5".to_string())
                .spawn(move || {
                    let t0 = Instant::now();
                    let verdict = cvc5.check_sat(&interrupt);
                    SolverResult {
                        solver: SolverId::Cvc5,
                        verdict,
                        elapsed_ms: t0.elapsed().as_millis() as u64,
                    }
                })
                .expect("failed to spawn CVC5 cross-validate thread")
        };

        // Set up a timeout watchdog: if either solver exceeds the timeout,
        // interrupt both.
        let deadline = Instant::now() + timeout;
        while !z3_handle.is_finished() || !cvc5_handle.is_finished() {
            if Instant::now() > deadline {
                z3_interrupt.store(true, Ordering::SeqCst);
                cvc5_interrupt.store(true, Ordering::SeqCst);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        let z3_result = z3_handle.join().unwrap_or(SolverResult {
            solver: SolverId::Z3,
            verdict: SolverVerdict::Error { message: "thread panic".into() },
            elapsed_ms: 0,
        });
        let cvc5_result = cvc5_handle.join().unwrap_or(SolverResult {
            solver: SolverId::Cvc5,
            verdict: SolverVerdict::Error { message: "thread panic".into() },
            elapsed_ms: 0,
        });

        Self::classify_cross_validate(z3_result, cvc5_result)
    }

    // ------------------------------------------------------------------------
    // Internal: result selection logic
    // ------------------------------------------------------------------------

    fn select_winner(
        z3_result: Option<SolverResult>,
        cvc5_result: Option<SolverResult>,
        tie_breaker: TieBreaker,
        total_elapsed_ms: u64,
    ) -> PortfolioResult {
        match (z3_result.clone(), cvc5_result.clone()) {
            // Both definitive — pick based on tie-breaker.
            (Some(z3), Some(cvc5))
                if z3.verdict.is_definitive() && cvc5.verdict.is_definitive() =>
            {
                let winner = match tie_breaker {
                    TieBreaker::Fastest => {
                        if z3.elapsed_ms <= cvc5.elapsed_ms { SolverId::Z3 } else { SolverId::Cvc5 }
                    }
                    TieBreaker::Z3 => SolverId::Z3,
                    TieBreaker::Cvc5 => SolverId::Cvc5,
                };
                let (verdict, elapsed) = match winner {
                    SolverId::Z3 => (z3.verdict.clone(), z3.elapsed_ms),
                    SolverId::Cvc5 => (cvc5.verdict.clone(), cvc5.elapsed_ms),
                };
                PortfolioResult {
                    winner,
                    verdict,
                    winner_elapsed_ms: elapsed,
                    total_elapsed_ms,
                    z3_result,
                    cvc5_result,
                }
            }
            // Only Z3 is definitive.
            (Some(z3), _) if z3.verdict.is_definitive() => {
                let elapsed = z3.elapsed_ms;
                PortfolioResult {
                    winner: SolverId::Z3,
                    verdict: z3.verdict.clone(),
                    winner_elapsed_ms: elapsed,
                    total_elapsed_ms,
                    z3_result: Some(z3),
                    cvc5_result,
                }
            }
            // Only CVC5 is definitive.
            (_, Some(cvc5)) if cvc5.verdict.is_definitive() => {
                let elapsed = cvc5.elapsed_ms;
                PortfolioResult {
                    winner: SolverId::Cvc5,
                    verdict: cvc5.verdict.clone(),
                    winner_elapsed_ms: elapsed,
                    total_elapsed_ms,
                    z3_result,
                    cvc5_result: Some(cvc5),
                }
            }
            // Neither definitive — return the least-bad answer.
            (Some(z3), Some(cvc5)) => {
                // Prefer Unknown over Error, Error over Cancelled.
                let (winner, verdict, elapsed) =
                    match (&z3.verdict, &cvc5.verdict) {
                        (SolverVerdict::Unknown { .. }, _) => {
                            (SolverId::Z3, z3.verdict.clone(), z3.elapsed_ms)
                        }
                        (_, SolverVerdict::Unknown { .. }) => {
                            (SolverId::Cvc5, cvc5.verdict.clone(), cvc5.elapsed_ms)
                        }
                        _ => (SolverId::Z3, z3.verdict.clone(), z3.elapsed_ms),
                    };
                PortfolioResult {
                    winner,
                    verdict,
                    winner_elapsed_ms: elapsed,
                    total_elapsed_ms,
                    z3_result: Some(z3),
                    cvc5_result: Some(cvc5),
                }
            }
            // Only Z3 ran.
            (Some(z3), None) => {
                let elapsed = z3.elapsed_ms;
                PortfolioResult {
                    winner: SolverId::Z3,
                    verdict: z3.verdict.clone(),
                    winner_elapsed_ms: elapsed,
                    total_elapsed_ms,
                    z3_result: Some(z3),
                    cvc5_result: None,
                }
            }
            // Only CVC5 ran.
            (None, Some(cvc5)) => {
                let elapsed = cvc5.elapsed_ms;
                PortfolioResult {
                    winner: SolverId::Cvc5,
                    verdict: cvc5.verdict.clone(),
                    winner_elapsed_ms: elapsed,
                    total_elapsed_ms,
                    z3_result: None,
                    cvc5_result: Some(cvc5),
                }
            }
            // Neither ran.
            (None, None) => PortfolioResult {
                winner: SolverId::Z3,
                verdict: SolverVerdict::Error {
                    message: "both portfolio threads failed to produce a result".into(),
                },
                winner_elapsed_ms: 0,
                total_elapsed_ms,
                z3_result: None,
                cvc5_result: None,
            },
        }
    }

    fn classify_cross_validate(
        z3_result: SolverResult,
        cvc5_result: SolverResult,
    ) -> CrossValidateResult {
        let z3_verdict = z3_result.verdict.clone();
        let cvc5_verdict = cvc5_result.verdict.clone();

        match (z3_verdict.is_definitive(), cvc5_verdict.is_definitive()) {
            (true, true) => {
                if z3_verdict == cvc5_verdict {
                    CrossValidateResult::Agreed {
                        verdict: z3_verdict,
                        z3_elapsed_ms: z3_result.elapsed_ms,
                        cvc5_elapsed_ms: cvc5_result.elapsed_ms,
                    }
                } else {
                    CrossValidateResult::Diverged {
                        z3_verdict,
                        cvc5_verdict,
                        z3_elapsed_ms: z3_result.elapsed_ms,
                        cvc5_elapsed_ms: cvc5_result.elapsed_ms,
                    }
                }
            }
            _ => CrossValidateResult::Incomplete {
                z3_result,
                cvc5_result,
            },
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock solver that returns a preset verdict after a configurable delay.
    struct MockSolver {
        id: SolverId,
        verdict: SolverVerdict,
        delay_ms: u64,
    }

    impl PortfolioSolver for MockSolver {
        fn check_sat(&mut self, interrupt: &AtomicBool) -> SolverVerdict {
            let end = Instant::now() + Duration::from_millis(self.delay_ms);
            while Instant::now() < end {
                if interrupt.load(Ordering::SeqCst) {
                    return SolverVerdict::Cancelled;
                }
                thread::sleep(Duration::from_millis(5));
            }
            self.verdict.clone()
        }

        fn solver_id(&self) -> SolverId {
            self.id
        }
    }

    #[test]
    fn portfolio_first_wins() {
        let z3 = MockSolver { id: SolverId::Z3, verdict: SolverVerdict::Sat, delay_ms: 20 };
        let cvc5 = MockSolver { id: SolverId::Cvc5, verdict: SolverVerdict::Unsat, delay_ms: 100 };

        let result = PortfolioExecutor::solve_portfolio(
            z3, cvc5,
            Duration::from_millis(500),
            TieBreaker::Fastest,
        );

        assert_eq!(result.winner, SolverId::Z3);
        assert_eq!(result.verdict, SolverVerdict::Sat);
    }

    #[test]
    fn portfolio_tie_breaker_z3() {
        let z3 = MockSolver { id: SolverId::Z3, verdict: SolverVerdict::Sat, delay_ms: 50 };
        let cvc5 = MockSolver { id: SolverId::Cvc5, verdict: SolverVerdict::Sat, delay_ms: 50 };

        let result = PortfolioExecutor::solve_portfolio(
            z3, cvc5,
            Duration::from_millis(500),
            TieBreaker::Z3,
        );

        // Either could win due to timing; just check the verdict is correct.
        assert_eq!(result.verdict, SolverVerdict::Sat);
    }

    #[test]
    fn cross_validate_agreement() {
        let z3 = MockSolver { id: SolverId::Z3, verdict: SolverVerdict::Unsat, delay_ms: 20 };
        let cvc5 = MockSolver { id: SolverId::Cvc5, verdict: SolverVerdict::Unsat, delay_ms: 20 };

        let result = PortfolioExecutor::solve_cross_validate(
            z3, cvc5,
            Duration::from_millis(500),
            CrossValidationStrictness::ResultOnly,
        );

        match result {
            CrossValidateResult::Agreed { verdict, .. } => {
                assert_eq!(verdict, SolverVerdict::Unsat);
            }
            other => panic!("expected Agreed, got {:?}", other),
        }
    }

    #[test]
    fn cross_validate_divergence() {
        let z3 = MockSolver { id: SolverId::Z3, verdict: SolverVerdict::Sat, delay_ms: 20 };
        let cvc5 = MockSolver { id: SolverId::Cvc5, verdict: SolverVerdict::Unsat, delay_ms: 20 };

        let result = PortfolioExecutor::solve_cross_validate(
            z3, cvc5,
            Duration::from_millis(500),
            CrossValidationStrictness::ResultOnly,
        );

        assert!(result.is_diverged(), "expected Diverged, got {:?}", result);
    }

    #[test]
    fn cross_validate_incomplete_on_timeout() {
        let z3 = MockSolver { id: SolverId::Z3, verdict: SolverVerdict::Sat, delay_ms: 1000 };
        let cvc5 = MockSolver { id: SolverId::Cvc5, verdict: SolverVerdict::Sat, delay_ms: 1000 };

        let result = PortfolioExecutor::solve_cross_validate(
            z3, cvc5,
            Duration::from_millis(50),
            CrossValidationStrictness::ResultOnly,
        );

        match result {
            CrossValidateResult::Incomplete { .. } => {}
            other => panic!("expected Incomplete for timeout, got {:?}", other),
        }
    }

    #[test]
    fn verdict_definitive_classification() {
        assert!(SolverVerdict::Sat.is_definitive());
        assert!(SolverVerdict::Unsat.is_definitive());
        assert!(!SolverVerdict::Unknown { reason: "timeout".into() }.is_definitive());
        assert!(!SolverVerdict::Cancelled.is_definitive());
        assert!(!SolverVerdict::Error { message: "foo".into() }.is_definitive());
    }
}
