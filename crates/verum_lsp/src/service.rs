//! One LSP method registry shared by CLI transports and embedded clients.
use crate::{
    Backend,
    refinement_validation::{
        InferRefinementParams, PromoteToCheckedParams, ValidateRefinementParams,
    },
};
use std::{future::Future, pin::Pin};
use tower_lsp::{LspService, jsonrpc::Result as JrResult};

/// `Pin<Box<dyn Future + Send>>`-returning wrapper around a `Backend::handle_*`
/// async method.
///
/// tower-lsp 0.20's `.custom_method` is bound by
/// `for<'a> Method<&'a S, P, R>`, which in turn requires a single concrete
/// `Fut: Future + Send` type. Passing an `async fn` method directly fails
/// the HRTB because `async fn` returns `impl Future + 'a` whose concrete
/// type varies with `&self`'s lifetime.
///
/// Boxing the future erases it into a named type per lifetime
/// (`Pin<Box<dyn Future + Send + 'a>>`) that satisfies both `Fn` and `Send`,
/// so `.custom_method` accepts it. The `Send` bound is satisfied because
/// `smt_worker` keeps every Z3 value off the handler's path — the
/// validator only holds an `SmtWorkerHandle` (`Send + Sync`).
macro_rules! boxed_handler {
    ($method:ident, $params:ty) => {{
        // Nameable free function so the returned `Pin<Box<dyn Future + 'a>>`
        // can explicitly relate its lifetime to `&'a Backend`. A bare closure
        // doesn't let us bind the two lifetimes, and Rust then complains with
        // "lifetime may not live long enough".
        fn handler<'a>(
            backend: &'a Backend,
            params: $params,
        ) -> Pin<Box<dyn Future<Output = JrResult<serde_json::Value>> + Send + 'a>> {
            Box::pin(backend.$method(params))
        }
        handler
    }};
}

/// Build an `LspService` with every custom `verum/*` JSON-RPC method wired
/// into the router.
///
/// tower-lsp's `LspService::new` only routes the standard LSP methods; any
/// custom name is silently dropped unless registered with `.custom_method`.
/// This helper is the single place that knows about Verum-specific requests
/// so the three transport entry points (stdio / TCP / named-pipe) stay in
/// sync and no method leaks as `MethodNotFound` at runtime.
pub fn service() -> (LspService<Backend>, tower_lsp::ClientSocket) {
    LspService::build(Backend::new)
        .custom_method(
            "verum/validateRefinement",
            boxed_handler!(handle_validate_refinement, ValidateRefinementParams),
        )
        .custom_method(
            "verum/promoteToChecked",
            boxed_handler!(handle_promote_to_checked, PromoteToCheckedParams),
        )
        .custom_method(
            "verum/inferRefinement",
            boxed_handler!(handle_infer_refinement, InferRefinementParams),
        )
        .custom_method(
            "verum/getProfile",
            boxed_handler!(handle_get_profile, serde_json::Value),
        )
        .custom_method(
            "verum/getEscapeAnalysis",
            boxed_handler!(handle_get_escape_analysis, serde_json::Value),
        )
        .custom_method(
            "verum/documentSnapshot",
            boxed_handler!(handle_document_snapshot, crate::project::SnapshotParams),
        )
        .custom_method(
            "verum/projectSnapshot",
            boxed_handler!(handle_project_snapshot, serde_json::Value),
        )
        .finish()
}
