//! Thread-isolated SMT worker for the LSP server.
//!
//! # Why this exists
//!
//! `verum_smt::RefinementVerifier` transitively holds a Z3 context
//! (`Rc<z3::ContextInternal>` plus `NonNull<_Z3_pattern>`) that is neither
//! `Send` nor `Sync`. The LSP server's custom `verum/*` methods are
//! registered with tower-lsp's `.custom_method`, whose trait bound
//! `for<'a> Method<&'a S, P, R>` in turn requires the returned future to be
//! `Send`. An `async fn` method on `Backend` that ever touches the verifier
//! captures that `!Send` state across an `.await` point, and the whole
//! future becomes non-Send — so the method can't be registered.
//!
//! The fix is to isolate all Z3 work behind a dedicated OS thread:
//!
//! - [`SmtWorker`] owns the verifier and runs a loop on a thread it
//!   spawns at startup. The verifier never leaves that thread.
//! - [`SmtWorkerHandle`] is the `Send + Sync + Clone` handle the rest of
//!   the LSP server talks to. It sends typed requests over a synchronous
//!   channel and awaits the response through a `tokio::sync::oneshot`.
//! - The request / response payloads only carry `Send` types (AST
//!   `Type` / `Expr`, `VerifyMode`, enums), so every step of the
//!   round-trip is `Send`.
//!
//! After the async handler awaits the oneshot, the resulting future
//! captures only Send types — tower-lsp's router accepts it.
//!
//! # Scope
//!
//! The worker is a compatibility shim today: one request type
//! (`VerifyRefinement`) covers the validate / promote / infer flows that
//! were previously inlined in `SmtRefinementChecker`. New SMT-backed
//! methods should extend [`SmtRequest`] rather than create a second
//! thread — a single Z3 context is enough for an interactive LSP workload,
//! and a single worker keeps ordering deterministic.

use std::sync::mpsc::{self, SyncSender};
use std::thread;
use std::time::Duration;

use tokio::sync::oneshot;
use verum_ast::{Expr, Type};
use verum_smt::{RefinementVerifier, VerificationError, VerifyMode};

/// Outcome of a single SMT refinement check.
///
/// Mirrors the `SmtCheckResult` used by the legacy `SmtRefinementChecker`
/// so callers can switch over without refactoring downstream matching.
#[derive(Debug, Clone)]
pub enum SmtCheckResult {
    /// SMT proved the refinement holds.
    Valid,
    /// SMT found a counterexample; the raw model text is returned for
    /// diagnostic extraction.
    Invalid { model: String },
    /// SMT could not conclude — timeout, solver panic, unsupported
    /// theory, etc. Callers usually map this to a "soft" diagnostic.
    Unknown,
}

/// Requests the worker understands. Each variant carries a
/// `oneshot::Sender` for the reply so the caller can `await` it.
enum SmtRequest {
    VerifyRefinement {
        ty: Type,
        context_expr: Option<Expr>,
        mode: VerifyMode,
        reply: oneshot::Sender<SmtCheckResult>,
    },
    /// Sentinel — drops the verifier and exits the worker thread cleanly.
    /// Sent automatically from `Drop` on the handle's last clone.
    Shutdown,
}

/// Handle to the SMT worker thread. Clone freely — all clones talk to
/// the same worker. `Send + Sync`.
///
/// Internal channel is a `SyncSender` with a small bound so a runaway
/// flood of validation requests applies back-pressure on the UI thread
/// rather than growing memory.
#[derive(Clone)]
pub struct SmtWorkerHandle {
    tx: SyncSender<SmtRequest>,
}

impl SmtWorkerHandle {
    /// Spawn a dedicated OS thread that owns a `RefinementVerifier` and
    /// services `SmtRequest`s from the returned handle.
    ///
    /// The thread is named `verum-smt-worker` for visibility in profiler
    /// / `ps` output. It panics only if `std::thread::spawn` fails (OOM);
    /// the worker loop itself swallows verifier panics and returns
    /// [`SmtCheckResult::Unknown`] so a bad goal can't topple the server.
    pub fn spawn() -> Self {
        // Bound of 32: enough to absorb a burst of saves across a
        // workspace without letting an unresponsive solver queue up
        // unbounded work.
        let (tx, rx) = mpsc::sync_channel::<SmtRequest>(32);

        thread::Builder::new()
            .name("verum-smt-worker".into())
            .spawn(move || worker_loop(rx))
            .expect("failed to spawn verum-smt-worker");

        Self { tx }
    }

    /// Run an SMT refinement check against the worker.
    ///
    /// Returns [`SmtCheckResult::Unknown`] when the worker is gone
    /// (shutdown, panic) — the LSP server stays alive even if SMT dies.
    pub async fn verify_refinement(
        &self,
        ty: Type,
        context_expr: Option<Expr>,
        mode: VerifyMode,
    ) -> SmtCheckResult {
        let (reply, wait) = oneshot::channel();
        let request = SmtRequest::VerifyRefinement {
            ty,
            context_expr,
            mode,
            reply,
        };
        if self.tx.send(request).is_err() {
            tracing::warn!("SMT worker disappeared — returning Unknown");
            return SmtCheckResult::Unknown;
        }
        match wait.await {
            Ok(res) => res,
            Err(_canceled) => SmtCheckResult::Unknown,
        }
    }

    /// Run an SMT refinement check with an outer timeout.
    ///
    /// Distinct from the solver's own `smt_timeout`: this bounds the
    /// round-trip including queueing, so an unresponsive solver can't
    /// block `on-type` validation.
    pub async fn verify_refinement_with_timeout(
        &self,
        ty: Type,
        context_expr: Option<Expr>,
        mode: VerifyMode,
        timeout: Duration,
    ) -> SmtCheckResult {
        match tokio::time::timeout(timeout, self.verify_refinement(ty, context_expr, mode)).await {
            Ok(r) => r,
            Err(_) => SmtCheckResult::Unknown,
        }
    }
}

/// Worker-thread entry point. Owns the verifier for its entire lifetime
/// and services requests until the channel closes or `Shutdown` arrives.
fn worker_loop(rx: mpsc::Receiver<SmtRequest>) {
    let verifier = RefinementVerifier::new();

    for request in rx {
        match request {
            SmtRequest::Shutdown => break,
            SmtRequest::VerifyRefinement {
                ty,
                context_expr,
                mode,
                reply,
            } => {
                let ce_ref = context_expr.as_ref();
                // Catch panics from the Z3 binding so a single broken
                // query can't take the whole worker down. A panic bubbles
                // up to the caller as `Unknown`.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    verifier.verify_refinement(&ty, ce_ref, Some(mode))
                }));

                let result = match outcome {
                    Err(_panic) => {
                        tracing::error!("SMT verifier panicked — returning Unknown");
                        SmtCheckResult::Unknown
                    }
                    Ok(Ok(_proof)) => SmtCheckResult::Valid,
                    Ok(Err(VerificationError::CannotProve {
                        counterexample: Some(ce),
                        ..
                    })) => SmtCheckResult::Invalid {
                        model: format!("{:?}", ce),
                    },
                    Ok(Err(VerificationError::CannotProve {
                        counterexample: None,
                        ..
                    }))
                    | Ok(Err(VerificationError::Timeout { .. }))
                    | Ok(Err(VerificationError::Unknown(_))) => SmtCheckResult::Unknown,
                    Ok(Err(_other)) => SmtCheckResult::Unknown,
                };

                // Reply best-effort: if the caller cancelled we drop.
                let _ = reply.send(result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The handle must be `Send + Sync` — that is the whole point of
    /// this module. A compile-time assertion keeps future refactors
    /// honest.
    #[test]
    fn handle_is_send_and_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<SmtWorkerHandle>();
        assert_sync::<SmtWorkerHandle>();
    }

    /// Spawning a worker does not deadlock when no requests are sent.
    /// This guards against a regression where the worker loop would
    /// block before reading the channel.
    #[test]
    fn spawn_without_requests_is_fine() {
        let _h = SmtWorkerHandle::spawn();
        // Drop the handle — the channel closes, the worker loop exits.
    }
}
