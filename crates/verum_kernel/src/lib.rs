//! # `verum_kernel` — Verum's LCF-style trusted kernel
//!
//! This crate is the **sole trusted checker** in Verum's verification
//! stack. All other components (elaborator, tactics, SMT backends,
//! cubical NbE, framework-axiom registry) produce proof terms in this
//! crate's [`CoreTerm`] language, and the kernel validates them against
//! a declared [`CoreType`]. If the kernel accepts a term, the user's
//! theorem is considered proved modulo the kernel plus whatever
//! explicitly-registered axioms were used (see [`AxiomRegistry`]).
//!
//! Target size: **under 5000 lines of Rust, audit-able by a single
//! reviewer in one session**. Everything that is not strictly required
//! for checking proof terms lives in other crates:
//!
//! - `verum_types`          — elaboration / inference (produces terms)
//! - `verum_verification`   — tactic evaluation (produces proof scripts)
//! - `verum_smt`            — SMT encoding + solver interface
//! - `verum_cbgr`           — memory-safety analyses
//! - `verum_vbc`            — bytecode codegen
//! - `verum_codegen`        — LLVM / MLIR lowering
//!
//! None of the above sit in the trusted computing base (TCB). They can
//! have bugs, and those bugs can only manifest as "the elaborator
//! refused a valid program" or "the SMT cert replay failed" — never as
//! "the kernel accepted a false theorem".
//!
//! ## Trusted Computing Base
//!
//! The authoritative TCB after this crate is complete:
//!
//! 1. The Rust compiler and its linked dependencies (unavoidable).
//! 2. This crate's [`check`] / [`verify`] loop and its subroutines.
//! 3. The axioms explicitly registered via [`AxiomRegistry::register`]
//!    (every registration records a framework name + citation so the
//!    TCB can be enumerated by `verum audit --framework-axioms`).
//!
//! Notably **outside** the TCB:
//!
//! - Z3 / CVC5 / E / Vampire / Alt-Ergo (any SMT backend) — their
//!   outputs arrive as [`SmtCertificate`] values and must be replayed
//!   by [`replay_smt_cert`] in this kernel.
//! - Any tactic, including the 22 built-in tactics — tactics produce
//!   [`CoreTerm`] values, which the kernel re-checks.
//! - The elaborator — a buggy elaborator can produce an ill-typed
//!   [`CoreTerm`], which the kernel will reject.
//!
//! ## Current status
//!
//! This file is the **skeleton** introduced when Verum's verification
//! architecture was driven to its ultimate form. The [`CoreTerm`] and
//! [`CoreType`] enums cover the shape of the explicit calculus; the
//! [`check`] routine is intentionally conservative and returns
//! [`KernelError::NotImplemented`] for constructs whose proof-term
//! checking is still being ported from `verum_types`. Full coverage
//! lands incrementally; every filled-in constructor is gated by a
//! dedicated unit test so the TCB grows strictly monotonically.
//!
//! The public API is the commitment: downstream code should compile
//! against this surface today, and incremental checker growth is
//! purely implementation-internal.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verum_common::{Heap, List, Maybe, Text};

/// Verum Unified Verification Architecture (VUVA) version stamp.
///
/// Closes B14 (#212). VFE §0.0 governance promises *"Каждое VFE-N
/// принятие — minor version bump VUVA"*; without a constant in code,
/// the version policy was unobservable. Tooling (CLI, certificate
/// emitters, cross-tool replay matrix per task #90) keys behaviour
/// on this constant.
///
/// **Bump policy** (per VFE §0.0 versioning):
///
///   * Major bump (`X` → `X+1`): backwards-incompatible changes to
///     [`CoreTerm`], [`KernelError`], or any `pub` kernel surface.
///   * Minor bump (`X.Y` → `X.Y+1`): VFE-N kernel-rule acceptance,
///     or any new optional `@require_extension` gating.
///   * Patch bump (`X.Y.Z` → `X.Y.Z+1`): bug fixes, soundness
///     tightening (e.g., the B4 saturation fix in commit 3b15c185),
///     refactoring without API change.
///
/// Current version reflects the V0/V1/V2 K-Eps-Mu rule + V1
/// K-Universe-Ascent rule + V0/V1 K-Refine-omega rule shipped
/// alongside the V1-V8 module split (#198) and B-series soundness
/// fixes. Bump on every kernel-rule addition.
pub const VUVA_VERSION: &str = "2.6.0";

pub mod proof_tree;

/// Kernel error type — split into its own module per #198 for
/// auditability of the trusted-base diagnostic surface. Re-exported
/// at crate root so external callers see the pre-split path
/// `verum_kernel::KernelError` unchanged.
pub mod errors;
pub use errors::KernelError;

/// Inductive-type registry + strict-positivity checking — split per
/// #198 (~355 LOC, the largest self-contained chunk in lib.rs). Hosts
/// `InductiveRegistry`, `RegisteredInductive`, `ConstructorSig`,
/// `PositivityCtx`, `check_strict_positivity` (K-Pos rule), plus the
/// UIP-shape detection helpers used by AxiomRegistry.
pub mod inductive;
pub use inductive::{
    ConstructorSig, InductiveRegistry, PositivityCtx, RegisteredInductive,
    check_strict_positivity,
};

/// Depth functions for kernel rules — split per #198. Hosts
/// `m_depth` (finite M-iteration depth, T-2f*), `m_depth_omega`
/// (ordinal modal-depth, T-2f***), `OrdinalDepth`, `check_refine_omega`
/// (K-Refine-omega rule entry point).
pub mod depth;
pub use depth::{OrdinalDepth, check_refine_omega, m_depth, m_depth_omega};

/// VFE-1 K-Eps-Mu kernel rule — split per #198. Hosts
/// `check_eps_mu_coherence` with V0/V1/V2 staging.
pub mod eps_mu;
pub use eps_mu::check_eps_mu_coherence;

/// VFE-3 K-Universe-Ascent kernel rule + UniverseTier — split per
/// #198. Hosts `UniverseTier` enum + `check_universe_ascent`.
pub mod universe_ascent;
pub use universe_ascent::{UniverseTier, check_universe_ascent};

/// Supporting kernel operations — `shape_of`, `substitute`,
/// `structural_eq`, `replay_smt_cert`. Split per #198. The
/// kernel's "infrastructure layer": these don't implement a
/// typing rule themselves but every rule in `infer` / `check`
/// calls one or more of them.
pub mod support;
pub use support::{replay_smt_cert, shape_of, structural_eq, substitute};

/// Axiom registry + AST loader — split per #198. Hosts
/// `AxiomRegistry`, `RegisteredAxiom`, `LoadAxiomsReport`, and
/// `load_framework_axioms`. UIP-shape axioms are syntactically
/// rejected to preserve cubical-univalence soundness.
pub mod axiom;
pub use axiom::{AxiomRegistry, LoadAxiomsReport, RegisteredAxiom, load_framework_axioms};

/// Kernel typing judgment — split per #198. Hosts the core LCF
/// `infer` function plus the `check` / `verify` / `verify_full`
/// shells callers use to gate proof admission.
pub mod infer;
pub use infer::{check, infer, verify, verify_full};

/// Core syntactic surface — `CoreTerm`, `CoreType`, `UniverseLevel`.
/// Split per #198 V7. The explicit calculus the kernel checks; every
/// other kernel module builds on top of these three types.
pub mod term;
pub use term::{CoreTerm, CoreType, UniverseLevel};

/// SMT certificate envelope — `SmtCertificate` +
/// `CERTIFICATE_SCHEMA_VERSION`. Split per #198 V7.
pub mod cert;
pub use cert::{CERTIFICATE_SCHEMA_VERSION, SmtCertificate};

/// Typing context + framework-axiom attribution — `Context` +
/// `FrameworkId`. Split per #198 V7.
pub mod ctx;
pub use ctx::{Context, FrameworkId};

