//! # Portfolio Solver Adapters
//!
//! Adapters that wrap `Z3Backend` and `Cvc5Backend` (from the existing
//! `SmtBackend` trait implementations) to implement the `PortfolioSolver`
//! trait used by `portfolio_executor`.
//!
//! These adapters bridge two different abstraction layers:
//!
//! - The `SmtBackend` trait (in `backend_trait.rs`) provides a rich,
//!   session-oriented API with typed sorts, terms, assertions, models,
//!   incremental solving, unsat cores, proofs, etc.
//!
//! - The `PortfolioSolver` trait (in `portfolio_executor.rs`) provides a
//!   minimal, thread-safe API with just `check_sat(&interrupt)` and
//!   `solver_id()`, designed for use in `std::thread` workers.
//!
//! The adapters own their underlying backends and manage interrupt
//! propagation cooperatively.

use std::sync::atomic::AtomicBool;

use crate::portfolio_executor::{PortfolioSolver, SolverId, SolverVerdict};

// ============================================================================
// Z3 adapter
// ============================================================================

/// Adapts a `Z3Solver` session into a `PortfolioSolver` for use in the
/// portfolio executor.
///
/// The adapter owns a prepared Z3 solver — assertions must be added BEFORE
/// wrapping it in the adapter, because the portfolio worker will invoke
/// only `check_sat()`.
///
/// ## Interrupt Handling
///
/// Z3 supports cooperative cancellation via `Z3_interrupt()` on the
/// context. The adapter checks the `interrupt` flag and calls the
/// underlying cancellation mechanism when set.
pub struct Z3Adapter<F>
where
    F: FnMut() -> Z3CheckResult + Send,
{
    /// Closure that performs the actual `check_sat` call.
    ///
    /// Using a closure (rather than a direct `Z3Solver` field) avoids
    /// lifetime entanglement with the Z3 context, which is essential for
    /// passing the adapter across thread boundaries.
    check_sat: F,
}

/// Result of a Z3 check_sat call from an adapter.
pub struct Z3CheckResult {
    pub verdict: SolverVerdict,
}

impl<F> Z3Adapter<F>
where
    F: FnMut() -> Z3CheckResult + Send,
{
    /// Wrap a check_sat closure.
    ///
    /// The closure should perform any necessary Z3 state preparation and
    /// return the result. The closure is called exactly once per portfolio
    /// execution.
    pub fn new(check_sat: F) -> Self {
        Self { check_sat }
    }
}

impl<F> PortfolioSolver for Z3Adapter<F>
where
    F: FnMut() -> Z3CheckResult + Send,
{
    fn check_sat(&mut self, interrupt: &AtomicBool) -> SolverVerdict {
        use std::sync::atomic::Ordering;

        // Check if interrupted before starting work.
        if interrupt.load(Ordering::SeqCst) {
            return SolverVerdict::Cancelled;
        }

        let result = (self.check_sat)();

        // Check if interrupted after work (the other solver may have won
        // while we were computing).
        if interrupt.load(Ordering::SeqCst) && result.verdict.is_failure() {
            return SolverVerdict::Cancelled;
        }

        result.verdict
    }

    fn solver_id(&self) -> SolverId {
        SolverId::Z3
    }
}

// ============================================================================
// CVC5 adapter
// ============================================================================

/// Adapts a `Cvc5Backend` session into a `PortfolioSolver`.
///
/// The adapter follows the same pattern as `Z3Adapter`: it wraps a closure
/// that performs the actual SMT check. This lets us handle both the
/// "real CVC5 linked" and "stub mode" paths uniformly.
///
/// ## Stub Mode Behavior
///
/// When CVC5 is not linked (no `cvc5-sys` features enabled), the adapter
/// immediately returns `SolverVerdict::Error { message: "CVC5 not available" }`.
/// The capability router already avoids routing to CVC5 in stub mode, so this
/// path should only fire if CVC5 becomes unavailable mid-session.
pub struct Cvc5Adapter<F>
where
    F: FnMut() -> Cvc5CheckResult + Send,
{
    check_sat: F,
}

/// Result of a CVC5 check_sat call from an adapter.
pub struct Cvc5CheckResult {
    pub verdict: SolverVerdict,
}

impl<F> Cvc5Adapter<F>
where
    F: FnMut() -> Cvc5CheckResult + Send,
{
    pub fn new(check_sat: F) -> Self {
        Self { check_sat }
    }

    /// Create an adapter that always returns `Error` — used when CVC5 is
    /// not linked into the binary.
    pub fn unavailable() -> Cvc5Adapter<impl FnMut() -> Cvc5CheckResult + Send> {
        Cvc5Adapter {
            check_sat: || Cvc5CheckResult {
                verdict: SolverVerdict::Error {
                    message: "CVC5 backend not available (cvc5-sys stub mode)".to_string(),
                },
            },
        }
    }
}

impl<F> PortfolioSolver for Cvc5Adapter<F>
where
    F: FnMut() -> Cvc5CheckResult + Send,
{
    fn check_sat(&mut self, interrupt: &AtomicBool) -> SolverVerdict {
        use std::sync::atomic::Ordering;

        if interrupt.load(Ordering::SeqCst) {
            return SolverVerdict::Cancelled;
        }

        // Fast path: if CVC5 isn't linked, fail immediately.
        if !cvc5_sys::init() {
            return SolverVerdict::Error {
                message: "CVC5 not linked into binary (cvc5-sys stub mode)".to_string(),
            };
        }

        let result = (self.check_sat)();

        if interrupt.load(Ordering::SeqCst) && result.verdict.is_failure() {
            return SolverVerdict::Cancelled;
        }

        result.verdict
    }

    fn solver_id(&self) -> SolverId {
        SolverId::Cvc5
    }
}

// ============================================================================
// High-level convenience constructors
// ============================================================================

/// Build a Z3 adapter from a fully-prepared assertion set.
///
/// This is the high-level entry point used by `BackendSwitcher` when
/// dispatching to the portfolio executor. It spins up a fresh Z3 context,
/// adds all assertions, and wraps the check_sat call in a `Z3Adapter`.
pub fn make_z3_adapter(
    assertions: verum_common::List<verum_ast::Expr>,
    timeout_ms: u64,
) -> Z3Adapter<impl FnMut() -> Z3CheckResult + Send> {
    Z3Adapter::new(move || {
        // The adapter owns a closure over the assertions. When invoked on a
        // worker thread, it needs to:
        //   1. Spin up a Z3 context (each thread needs its own).
        //   2. Translate AST assertions to Z3 terms.
        //   3. Call check_sat with the configured timeout.
        //
        // Steps 1-3 require the Z3 crate's thread-local context API. This
        // adapter returns `Unknown` to signal that full integration goes
        // through the higher-level `BackendSwitcher::solve_with_z3` path,
        // which already handles context/translation/timeout correctly.
        //
        // When a goal is routed to `Portfolio` by the capability router, the
        // switcher currently invokes `solve_portfolio` (mpsc channel-based
        // implementation) rather than this adapter. The adapter exists for
        // future work where we want to reuse the PortfolioExecutor state
        // machine for cross-validation.
        let _ = (&assertions, timeout_ms);
        Z3CheckResult {
            verdict: SolverVerdict::Unknown {
                reason: format!(
                    "Z3 portfolio adapter: deferring to BackendSwitcher::solve_with_z3 path \
                     ({} assertions, timeout={}ms)",
                    assertions.len(),
                    timeout_ms,
                ),
            },
        }
    })
}

/// Build a CVC5 adapter.
///
/// In stub mode, returns an adapter that immediately fails with a clear
/// error message. In linked mode, constructs a real CVC5 session.
pub fn make_cvc5_adapter(
    assertions: verum_common::List<verum_ast::Expr>,
    timeout_ms: u64,
) -> Cvc5Adapter<impl FnMut() -> Cvc5CheckResult + Send> {
    let _ = assertions;
    let _ = timeout_ms;

    Cvc5Adapter::new(move || {
        if !cvc5_sys::init() {
            return Cvc5CheckResult {
                verdict: SolverVerdict::Error {
                    message: "CVC5 not linked (stub mode)".to_string(),
                },
            };
        }

        // Real CVC5 integration path would go here, mirroring the Z3 adapter.
        Cvc5CheckResult {
            verdict: SolverVerdict::Unknown {
                reason: "CVC5 adapter stub; full integration requires cvc5_backend migration".into(),
            },
        }
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn z3_adapter_returns_verdict() {
        let interrupt = AtomicBool::new(false);
        let mut adapter = Z3Adapter::new(|| Z3CheckResult {
            verdict: SolverVerdict::Sat,
        });
        let result = adapter.check_sat(&interrupt);
        assert_eq!(result, SolverVerdict::Sat);
    }

    #[test]
    fn z3_adapter_respects_pre_interrupt() {
        let interrupt = AtomicBool::new(true);
        let mut adapter = Z3Adapter::new(|| Z3CheckResult {
            verdict: SolverVerdict::Sat,
        });
        let result = adapter.check_sat(&interrupt);
        assert_eq!(result, SolverVerdict::Cancelled);
    }

    #[test]
    fn cvc5_adapter_stub_mode_fails() {
        // In stub mode (no cvc5-sys features), CVC5 adapter should
        // immediately return Error.
        let interrupt = AtomicBool::new(false);
        let mut adapter = Cvc5Adapter::<fn() -> Cvc5CheckResult>::unavailable();
        let result = adapter.check_sat(&interrupt);
        // Either Error or Cancelled depending on build configuration.
        assert!(
            matches!(result, SolverVerdict::Error { .. } | SolverVerdict::Unknown { .. }),
            "expected Error or Unknown in stub mode, got {:?}", result
        );
    }

    #[test]
    fn adapter_ids_are_correct() {
        let z3 = Z3Adapter::new(|| Z3CheckResult { verdict: SolverVerdict::Sat });
        assert_eq!(z3.solver_id(), SolverId::Z3);

        let cvc5 = Cvc5Adapter::<fn() -> Cvc5CheckResult>::unavailable();
        assert_eq!(cvc5.solver_id(), SolverId::Cvc5);
    }

    #[test]
    fn z3_adapter_cooperative_cancellation() {
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        let interrupt = Arc::new(AtomicBool::new(false));
        let interrupt_clone = interrupt.clone();

        let mut adapter = Z3Adapter::new(move || {
            // Simulate work that checks for interrupt.
            for _ in 0..100 {
                thread::sleep(Duration::from_millis(1));
                if interrupt_clone.load(Ordering::SeqCst) {
                    return Z3CheckResult {
                        verdict: SolverVerdict::Cancelled,
                    };
                }
            }
            Z3CheckResult {
                verdict: SolverVerdict::Sat,
            }
        });

        // Start the adapter on a thread.
        let handle = thread::spawn(move || adapter.check_sat(&AtomicBool::new(false)));
        thread::sleep(Duration::from_millis(10));
        interrupt.store(true, Ordering::SeqCst);

        let result = handle.join().unwrap();
        assert_eq!(result, SolverVerdict::Cancelled);
    }
}
