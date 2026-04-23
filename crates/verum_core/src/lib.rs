//! Verum Core IR.
//!
//! A typed intermediate representation shared across the verification
//! pipeline. Sits between the raw AST (`verum_ast`) and the LCF-style
//! trusted kernel (`verum_kernel`):
//!
//! ```text
//!   verum_ast          — surface syntax, lossless
//!   verum_core (here)  — typed IR, pipeline-shared
//!   verum_kernel       — LCF core terms, trusted
//! ```
//!
//! ## Why a dedicated crate
//!
//! Before this crate landed, the verification pipeline threaded raw
//! `verum_ast::Expr` values through every stage. That had two problems:
//!
//! 1. **Pipeline instability.** Every change to the AST broke every
//!    downstream pass. Verification-relevant information (refinement
//!    predicates, variant tags, framework attribution) was scattered
//!    across the AST with no unified access API.
//!
//! 2. **No stable contract between stages.** The tactic layer, SMT
//!    translator, and proof-certificate exporter each had to rediscover
//!    the same IR-level facts (e.g. "this expression is a variant
//!    constructor reference") by independent analysis.
//!
//! `verum_core` establishes the stable contract: a small set of
//! **pipeline-shared typed nodes** plus accessor APIs. Individual
//! consumers (SMT translator, proof engine, kernel-replay) project into
//! their own representations, but the *shared contract* lives here.
//!
//! ## Scope
//!
//! This is Phase 1.1 (the skeleton) — the crate intentionally stays
//! small. It defines:
//!
//! * [`IrExpr`] — shared typed-expression form with stable accessors.
//! * [`IrType`] — shared typed-type form.
//! * [`IrObligation`] — a proof obligation with its hypothesis context,
//!   goal, and metadata.
//! * [`IrModule`] — a read-only view of a module's IR-level contents
//!   (functions, types, theorems, axioms).
//!
//! Phase 1.2 will add the lowering passes from `verum_ast::Module` to
//! `IrModule`, replacing the ad-hoc walks currently scattered across
//! `verum_compiler::verify_cmd` and `verum_smt`.
//!
//! ## Design principles
//!
//! * **No stdlib hardcoding.** The IR knows nothing about `Maybe`,
//!   `Result`, `List`, etc. — they're ordinary user-level types as far
//!   as the IR is concerned.
//! * **Serializable.** All IR nodes derive `Serialize`/`Deserialize` so
//!   the IR can be persisted (cache, incremental compilation, remote
//!   verification, LSP state).
//! * **Span-preserved.** Every node carries its source span for
//!   diagnostics.
//! * **Immutable after construction.** Lowering produces an `IrModule`
//!   that verification passes consume without mutation.

#![deny(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verum_common::Text;
use verum_common::span::Span;

pub mod expr;
pub mod ty;
pub mod obligation;
pub mod module;

pub use expr::IrExpr;
pub use ty::IrType;
pub use obligation::IrObligation;
pub use module::{IrFunction, IrModule, IrTheorem, IrTypeDecl, IrVariantKind};

/// Errors raised by IR construction / projection.
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum IrError {
    /// A referenced identifier could not be resolved in the module's
    /// symbol table.
    #[error("IR: unresolved name '{name}' at {span:?}")]
    UnresolvedName {
        /// The name that failed to resolve.
        name: Text,
        /// Source location of the reference.
        span: Span,
    },

    /// A type-level expression could not be lowered to a typed form.
    #[error("IR: untypeable expression at {span:?} ({reason})")]
    Untypeable {
        /// Human-readable reason.
        reason: Text,
        /// Source location of the offending AST node.
        span: Span,
    },
}

/// Shorthand for `Result<T, IrError>`.
pub type IrResult<T> = Result<T, IrError>;

/// Pipeline version string. Bumped when the IR contract changes in a
/// way that invalidates persisted caches.
pub const IR_VERSION: &str = "0.1.0";
