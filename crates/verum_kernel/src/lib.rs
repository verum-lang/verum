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

// =============================================================================
// CoreTerm — the explicit calculus
// =============================================================================

/// A term in Verum-Core, the explicit typed calculus the kernel checks.
///
/// This is the minimum syntactic surface the kernel needs in order to
/// proof-check every Verum feature: dependent functions (Π), dependent
/// pairs (Σ), cubical path types with [`HComp`] / [`Transp`] / [`Glue`],
/// refinement types with SMT-discharged predicates, user / stdlib /
/// higher inductive types, and framework axioms.
///
/// Surface-level constructs (match expressions, structured Isar-style
/// proofs, automated tactics, …) elaborate to these terms in
/// `verum_types` before they reach the kernel.
///
/// [`HComp`]: CoreTerm::HComp
/// [`Transp`]: CoreTerm::Transp
/// [`Glue`]: CoreTerm::Glue
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CoreTerm {
    /// A de Bruijn / name variable reference.
    Var(Text),

    /// Universe `Type(i)` for level `i`, or `Prop` (propositional universe).
    Universe(UniverseLevel),

    /// Dependent function type: `Pi (x : dom) . codom`.
    Pi {
        /// The bound name (used for pretty-printing; kernel uses de Bruijn).
        binder: Text,
        /// Domain type.
        domain: Heap<CoreTerm>,
        /// Codomain term (may reference `binder`).
        codomain: Heap<CoreTerm>,
    },

    /// Lambda abstraction: `λ (x : domain) . body`.
    Lam {
        /// The bound name.
        binder: Text,
        /// Parameter type.
        domain: Heap<CoreTerm>,
        /// Body term.
        body: Heap<CoreTerm>,
    },

    /// Function application: `f a`.
    App(Heap<CoreTerm>, Heap<CoreTerm>),

    /// Dependent pair type: `Sigma (x : fst_ty) . snd_ty`.
    Sigma {
        /// The bound name.
        binder: Text,
        /// First component type.
        fst_ty: Heap<CoreTerm>,
        /// Second component type (may reference `binder`).
        snd_ty: Heap<CoreTerm>,
    },

    /// Dependent pair constructor: `(a, b)`.
    Pair(Heap<CoreTerm>, Heap<CoreTerm>),

    /// First projection: `fst p`.
    Fst(Heap<CoreTerm>),

    /// Second projection: `snd p`.
    Snd(Heap<CoreTerm>),

    /// Cubical path type: `Path<A>(a, b)`.
    PathTy {
        /// Carrier type.
        carrier: Heap<CoreTerm>,
        /// Left endpoint.
        lhs: Heap<CoreTerm>,
        /// Right endpoint.
        rhs: Heap<CoreTerm>,
    },

    /// Reflexivity path: `refl(a) : Path<A>(a, a)`.
    Refl(Heap<CoreTerm>),

    /// Cubical homogeneous composition: `hcomp φ walls base`.
    HComp {
        /// Face / interval formula.
        phi: Heap<CoreTerm>,
        /// Walls: a family over the face.
        walls: Heap<CoreTerm>,
        /// Base element at `i0`.
        base: Heap<CoreTerm>,
    },

    /// Cubical transport: `transp(p, r, t)`.
    Transp {
        /// Path in `Type`.
        path: Heap<CoreTerm>,
        /// Regularity interval endpoint.
        regular: Heap<CoreTerm>,
        /// Source value.
        value: Heap<CoreTerm>,
    },

    /// Glue type: `Glue<A>(φ, T, e)` for computational univalence.
    Glue {
        /// Carrier type.
        carrier: Heap<CoreTerm>,
        /// Face formula.
        phi: Heap<CoreTerm>,
        /// Fiber type family.
        fiber: Heap<CoreTerm>,
        /// Equivalence family.
        equiv: Heap<CoreTerm>,
    },

    /// Refinement type: `{ x : base | predicate(x) }`.
    ///
    /// The kernel records the predicate but delegates decidability to
    /// the SMT backend via [`CoreTerm::SmtProof`].
    Refine {
        /// Base type.
        base: Heap<CoreTerm>,
        /// Bound variable inside the predicate.
        binder: Text,
        /// Predicate term — must have type `Bool`.
        predicate: Heap<CoreTerm>,
    },

    /// Reference to a named inductive / higher-inductive type (from
    /// the stdlib, from the user's project, or from a framework axiom).
    ///
    /// The kernel looks `path` up in its registry of declared type
    /// constructors; missing names are a kernel error, not silent
    /// pass-through.
    Inductive {
        /// Qualified type name.
        path: Text,
        /// Type-level arguments.
        args: List<CoreTerm>,
    },

    /// Inductive eliminator: `elim e motive cases`.
    Elim {
        /// Scrutinee.
        scrutinee: Heap<CoreTerm>,
        /// Motive predicate.
        motive: Heap<CoreTerm>,
        /// One branch per constructor.
        cases: List<CoreTerm>,
    },

    /// A proof term produced by an SMT backend that the kernel will
    /// replay via [`replay_smt_cert`]. The certificate is not trusted
    /// by itself — the kernel re-derives a [`CoreTerm`] from it.
    SmtProof(SmtCertificate),

    /// A trusted postulate registered via [`AxiomRegistry::register`].
    ///
    /// Every `Axiom` node records the framework name + citation so the
    /// TCB can be enumerated by `verum audit --framework-axioms`.
    Axiom {
        /// Axiom identifier.
        name: Text,
        /// Claimed type (the kernel does not re-check this — it is the
        /// axiom).
        ty: Heap<CoreTerm>,
        /// Framework attribution.
        framework: FrameworkId,
    },

    /// VFE-1 V0 — `EpsilonOf(α)` represents the canonical enactment
    /// image of an articulation under the M ⊣ A biadjunction (the
    /// activation modality applied at the articulation level). The
    /// kernel uses this constructor to track the natural-equivalence
    /// τ : ε ∘ M ≃ A ∘ ε of Proposition 5.1 / Corollary 5.10.
    ///
    /// V0 ships the constructor + `K-Eps-Mu` skeleton (see
    /// `Kernel::check_eps_mu_coherence`); the full naturality check
    /// is deferred to V1, where the τ-witness construction will be
    /// wired in.
    EpsilonOf(Heap<CoreTerm>),

    /// VFE-1 V0 — `AlphaOf(ε)` represents the canonical articulation
    /// image of an enactment (the inverse direction of the M ⊣ A
    /// biadjunction). Together with `EpsilonOf` this enables kernel-
    /// level reasoning about the ε ↔ α duality.
    AlphaOf(Heap<CoreTerm>),

    /// VFE-7 V1 — `ModalBox(φ)` represents `□φ` (necessity in the
    /// underlying modal logic). md^ω(□φ) = md^ω(φ) + 1 per
    /// Definition 136.D1. The K-Refine-omega rule uses the
    /// resulting ordinal to gate refinement-type formation.
    ModalBox(Heap<CoreTerm>),

    /// VFE-7 V1 — `ModalDiamond(φ)` represents `◇φ` (possibility).
    /// md^ω(◇φ) = md^ω(φ) + 1 per Definition 136.D1.
    ModalDiamond(Heap<CoreTerm>),

    /// VFE-7 V1 — `ModalBigAnd(P_0, ..., P_κ)` represents the
    /// transfinite conjunction `⋀_{i<κ} P_i`. md^ω of the big-and
    /// is the *supremum* of the components' md^ω-ranks, per
    /// Definition 136.D1 + Lemma 136.L0 well-founded ordinal
    /// recursion. Used to express modal axiom schemes that
    /// quantify over all possible-world labels at once.
    ModalBigAnd(List<Heap<CoreTerm>>),
}

// =============================================================================
// CoreType — simplified surface for kernel diagnostics
// =============================================================================

/// A structural view of a type used by [`check`] diagnostics.
///
/// Internally, the kernel works directly with [`CoreTerm`] values in
/// `Universe` position. This enum exists so that errors like
/// "expected a Pi type, got Int" can be reported without copying large
/// [`CoreTerm`] trees around.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoreType {
    /// A universe `Type(i)` or `Prop`.
    Universe(UniverseLevel),
    /// A Π-type head.
    Pi,
    /// A Σ-type head.
    Sigma,
    /// A cubical path-type head.
    Path,
    /// A refinement-type head.
    Refine,
    /// A glue-type head.
    Glue,
    /// A named inductive / user / HIT head.
    Inductive(Text),
    /// Any other shape — deliberately coarse.
    Other,
}

/// A universe level — concrete, variable, or successor of another level.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UniverseLevel {
    /// `Type(n)` for a concrete non-negative `n`.
    Concrete(u32),
    /// `Type(u)` for a level variable `u`.
    Variable(Text),
    /// `u + 1`.
    Succ(Heap<UniverseLevel>),
    /// `max(u, v)`.
    Max(Heap<UniverseLevel>, Heap<UniverseLevel>),
    /// The propositional universe `Prop`.
    Prop,
}

// =============================================================================
// SMT certificate — replay surface
// =============================================================================

/// A proof certificate produced by an SMT backend.
///
/// The kernel consumes this via [`replay_smt_cert`] and reconstructs a
/// [`CoreTerm`] witness. This is the primary mechanism that takes Z3 /
/// CVC5 / E / Vampire / Alt-Ergo **out of the TCB**: a bug in a solver
/// that produced a spurious "proof" will fail the replay here, not
/// leak into accepted theorems.
///
/// The certificate format is backend-neutral: each backend's native
/// proof trace is normalized into the common shape by
/// `verum_smt::proof_extraction` before landing here.
///
/// # Envelope versioning
///
/// `schema_version` identifies the certificate envelope format. The
/// kernel rejects any certificate whose `schema_version` is greater
/// than [`CERTIFICATE_SCHEMA_VERSION`] — this lets forward
/// compatibility be negotiated explicitly rather than silently
/// accepting unknown-shape envelopes. Version `0` is treated as
/// "legacy unversioned" for backward compatibility with pre-envelope
/// certificates on disk.
///
/// # Metadata
///
/// `metadata` is a free-form key/value store for non-trust-relevant
/// annotations (tactics used, solver options, timing, obligation
/// provenance, …). The kernel never reads these fields — they are
/// carried end-to-end so tooling (`verum audit --framework-axioms`,
/// proof export, cross-tool replay) can preserve diagnostic context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtCertificate {
    /// Envelope schema version. Zero means "legacy unversioned";
    /// current shipping version is [`CERTIFICATE_SCHEMA_VERSION`].
    #[serde(default)]
    pub schema_version: u32,
    /// Which backend produced the certificate (for routing the replay).
    pub backend: Text,
    /// Backend version — certificates are keyed by version because
    /// different solver versions can have different proof-rule sets.
    pub backend_version: Text,
    /// Serialized proof trace. Format is backend-specific; the replay
    /// routine knows how to parse each known backend.
    pub trace: List<u8>,
    /// Hash of the obligation so callers can cross-check that the
    /// certificate belongs to the goal they were trying to prove.
    pub obligation_hash: Text,
    /// Verum compiler version that produced the certificate. Used by
    /// the cross-tool replay matrix (task #90) to key CI runs.
    #[serde(default)]
    pub verum_version: Text,
    /// ISO-8601 timestamp of certificate creation (UTC). Allows
    /// disk-cached certificates to be invalidated by age without
    /// re-hashing.
    #[serde(default)]
    pub created_at: Text,
    /// Free-form non-trust-relevant annotations. Not inspected by the
    /// kernel.
    #[serde(default)]
    pub metadata: List<(Text, Text)>,
}

/// Current SmtCertificate envelope schema version.
///
/// Bump this constant whenever the envelope shape changes
/// incompatibly. The kernel rejects any certificate whose
/// `schema_version` exceeds this value, which gives tooling a clean
/// error path on version skew.
pub const CERTIFICATE_SCHEMA_VERSION: u32 = 1;

impl SmtCertificate {
    /// Construct a new certificate with the current schema version
    /// and [`verum_version`] filled in from the crate metadata.
    ///
    /// `created_at` is left empty; callers that want timestamps
    /// should populate them via [`with_created_at`] (the kernel
    /// crate is intentionally free of `chrono`/`std::time::SystemTime`
    /// dependencies to keep the TCB minimal).
    ///
    /// [`verum_version`]: Self::verum_version
    /// [`with_created_at`]: Self::with_created_at
    pub fn new(
        backend: Text,
        backend_version: Text,
        trace: List<u8>,
        obligation_hash: Text,
    ) -> Self {
        Self {
            schema_version: CERTIFICATE_SCHEMA_VERSION,
            backend,
            backend_version,
            trace,
            obligation_hash,
            verum_version: Text::from(env!("CARGO_PKG_VERSION")),
            created_at: Text::new(),
            metadata: List::new(),
        }
    }

    /// Attach an ISO-8601 timestamp to the certificate. The kernel
    /// does not parse this field — it is carried end-to-end for
    /// tooling use.
    pub fn with_created_at(mut self, ts: Text) -> Self {
        self.created_at = ts;
        self
    }

    /// Attach a single metadata key/value pair. See the struct-level
    /// docs for what metadata is used for.
    pub fn with_metadata(mut self, key: Text, value: Text) -> Self {
        self.metadata.push((key, value));
        self
    }

    /// Validate the envelope schema. Returns [`Err`] if the schema
    /// version is newer than this kernel build understands.
    ///
    /// Version `0` is accepted as "legacy unversioned" for backward
    /// compatibility with pre-1.0 on-disk certificates.
    pub fn validate_schema(&self) -> Result<(), KernelError> {
        if self.schema_version > CERTIFICATE_SCHEMA_VERSION {
            return Err(KernelError::UnsupportedCertificateSchema {
                found: self.schema_version,
                max_supported: CERTIFICATE_SCHEMA_VERSION,
            });
        }
        Ok(())
    }
}

// =============================================================================
// Framework-axiom attribution
// =============================================================================

/// A stable identifier for an external mathematical framework whose
/// theorems Verum postulates as axioms.
///
/// Every registered axiom carries one of these so `verum audit
/// --framework-axioms` can enumerate the exact set of external results
/// (Lurie HTT, Schreiber DCCT, Connes reconstruction, Petz
/// classification, Arnold-Mather catastrophe, Baez-Dolan coherence, …)
/// on which any given proof relies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameworkId {
    /// Short machine-readable framework identifier,
    /// e.g. `"lurie_htt"`, `"schreiber_dcct"`, `"connes_reconstruction"`.
    pub framework: Text,
    /// Citation string pointing at the specific result,
    /// e.g. `"HTT 6.2.2.7"`, `"DCCT §3.9"`, `"Connes 2008 axiom (vii)"`.
    pub citation: Text,
}

// =============================================================================
// Context — de Bruijn environment
// =============================================================================

/// The typing context maintained during checking.
///
/// Each binding maps a name to its declared type. The kernel never
/// performs inference — it only checks.
#[derive(Debug, Clone, Default)]
pub struct Context {
    bindings: List<(Text, CoreTerm)>,
}

impl Context {
    /// An empty context.
    pub fn new() -> Self {
        Self { bindings: List::new() }
    }

    /// Extend the context with a new typed binding. Shadowing is
    /// allowed and mirrors surface semantics.
    pub fn extend(&self, name: Text, ty: CoreTerm) -> Self {
        let mut fresh = self.clone();
        fresh.bindings.push((name, ty));
        fresh
    }

    /// Look up the type of a variable. Returns the innermost binding
    /// (shadowing-respecting).
    pub fn lookup(&self, name: &str) -> Maybe<&CoreTerm> {
        for (n, ty) in self.bindings.iter().rev() {
            if n.as_str() == name {
                return Maybe::Some(ty);
            }
        }
        Maybe::None
    }

    /// Number of bindings currently in scope.
    pub fn depth(&self) -> usize {
        self.bindings.len()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_ty() -> CoreTerm {
        CoreTerm::Inductive {
            path: Text::from("Unit"),
            args: List::new(),
        }
    }

    #[test]
    fn empty_context_depth_zero() {
        assert_eq!(Context::new().depth(), 0);
    }

    #[test]
    fn extend_then_lookup_finds_binding() {
        let ctx = Context::new().extend(Text::from("x"), unit_ty());
        assert!(matches!(ctx.lookup("x"), Maybe::Some(_)));
        assert_eq!(ctx.depth(), 1);
    }

    #[test]
    fn shadow_returns_innermost() {
        let ctx = Context::new()
            .extend(Text::from("x"), unit_ty())
            .extend(
                Text::from("x"),
                CoreTerm::Universe(UniverseLevel::Concrete(0)),
            );
        match ctx.lookup("x") {
            Maybe::Some(ty) => assert!(matches!(
                ty,
                CoreTerm::Universe(UniverseLevel::Concrete(0))
            )),
            Maybe::None => panic!("expected shadowed binding"),
        }
    }

    #[test]
    fn unbound_variable_is_an_error() {
        let ctx = Context::new();
        let ax = AxiomRegistry::new();
        let err = check(&ctx, &CoreTerm::Var(Text::from("y")), &ax).unwrap_err();
        assert!(matches!(err, KernelError::UnboundVariable(_)));
    }

    #[test]
    fn universe_checks_to_universe() {
        // `Type(0) : Type(1)` — under predicative universes, every level
        // inhabits its strict successor, so `check` (shape-head projection
        // over `infer`) reports the successor level, not the input.
        let ctx = Context::new();
        let ax = AxiomRegistry::new();
        let ty = check(
            &ctx,
            &CoreTerm::Universe(UniverseLevel::Concrete(0)),
            &ax,
        )
        .unwrap();
        assert_eq!(ty, CoreType::Universe(UniverseLevel::Concrete(1)));
    }

    #[test]
    fn axiom_registry_refuses_duplicate_name() {
        let mut reg = AxiomRegistry::new();
        let fw = FrameworkId {
            framework: Text::from("lurie_htt"),
            citation: Text::from("HTT 6.2.2.7"),
        };
        reg.register(Text::from("sheafification"), unit_ty(), fw.clone())
            .unwrap();
        let err = reg
            .register(Text::from("sheafification"), unit_ty(), fw)
            .unwrap_err();
        assert!(matches!(err, KernelError::DuplicateAxiom(_)));
    }

    #[test]
    fn axiom_known_to_registry_is_checkable() {
        let mut reg = AxiomRegistry::new();
        let fw = FrameworkId {
            framework: Text::from("connes_reconstruction"),
            citation: Text::from("Connes 2008 axiom (vii)"),
        };
        reg.register(Text::from("first_order_condition"), unit_ty(), fw.clone())
            .unwrap();
        let term = CoreTerm::Axiom {
            name: Text::from("first_order_condition"),
            ty: Heap::new(unit_ty()),
            framework: fw,
        };
        let ctx = Context::new();
        let head = check(&ctx, &term, &reg).unwrap();
        // The registered axiom has `Unit` as its type; `infer` returns
        // that type verbatim and `shape_of` projects it to the
        // `Inductive(_)` head.
        assert_eq!(head, CoreType::Inductive(Text::from("Unit")));
    }

    #[test]
    fn smt_replay_rejects_empty_trace() {
        let cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            List::new(),
            Text::from("sha256:0"),
        );
        let ctx = Context::new();
        let err = replay_smt_cert(&ctx, &cert).unwrap_err();
        assert!(matches!(err, KernelError::EmptyCertificate));
    }

    #[test]
    fn smt_replay_rejects_unknown_backend() {
        let mut trace = List::new();
        trace.push(0x01);
        let cert = SmtCertificate::new(
            Text::from("unknown_solver"),
            Text::from("1.0.0"),
            trace,
            Text::from("sha256:0"),
        );
        let ctx = Context::new();
        let err = replay_smt_cert(&ctx, &cert).unwrap_err();
        assert!(matches!(err, KernelError::UnknownBackend(_)));
    }

    #[test]
    fn smt_replay_rejects_unknown_rule_tag() {
        let mut trace = List::new();
        trace.push(0xFF);
        let cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            trace,
            Text::from("sha256:a"),
        );
        let ctx = Context::new();
        let err = replay_smt_cert(&ctx, &cert).unwrap_err();
        assert!(matches!(err, KernelError::UnknownRule { tag: 0xFF, .. }));
    }

    #[test]
    fn smt_replay_rejects_missing_obligation_hash() {
        let mut trace = List::new();
        trace.push(0x03);
        let cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            trace,
            Text::from(""),
        );
        let ctx = Context::new();
        let err = replay_smt_cert(&ctx, &cert).unwrap_err();
        assert!(matches!(err, KernelError::MissingObligationHash));
    }

    #[test]
    fn smt_replay_refl_tag_produces_axiom_witness() {
        let mut trace = List::new();
        trace.push(0x01);
        let cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            trace,
            Text::from("sha256:deadbeef"),
        );
        let ctx = Context::new();
        let term = replay_smt_cert(&ctx, &cert).unwrap();
        match term {
            CoreTerm::Axiom { name, framework, .. } => {
                assert!(name.as_str().starts_with("smt_cert:z3:refl:"));
                assert_eq!(framework.framework.as_str(), "z3:refl");
                assert_eq!(framework.citation.as_str(), "sha256:deadbeef");
            }
            other => panic!("expected Axiom, got {:?}", other),
        }
    }

    #[test]
    fn smt_replay_accepts_cvc5_smt_unsat_tag() {
        let mut trace = List::new();
        trace.push(0x03);
        let cert = SmtCertificate::new(
            Text::from("cvc5"),
            Text::from("1.2.0"),
            trace,
            Text::from("sha256:feed"),
        );
        let ctx = Context::new();
        let term = replay_smt_cert(&ctx, &cert).unwrap();
        match term {
            CoreTerm::Axiom { framework, .. } => {
                assert_eq!(framework.framework.as_str(), "cvc5:smt_unsat");
            }
            other => panic!("expected Axiom, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------
    // SmtProof constructor tests (task #62)
    // -----------------------------------------------------------------

    #[test]
    fn smtproof_infer_replays_certificate_and_returns_bool_type() {
        let mut trace = List::new();
        trace.push(0x03); // smt_unsat tag
        let cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            trace,
            Text::from("sha256:deadbeef"),
        );
        let term = CoreTerm::SmtProof(cert);
        let ctx = Context::new();
        let reg = AxiomRegistry::new();
        let ty = infer(&ctx, &term, &reg).unwrap();
        assert_eq!(
            ty,
            CoreTerm::Inductive {
                path: Text::from("Bool"),
                args: List::new(),
            }
        );
    }

    #[test]
    fn smtproof_infer_rejects_malformed_certificate() {
        // Empty trace → replay fails → infer surfaces EmptyCertificate.
        let cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            List::new(),
            Text::from("sha256:x"),
        );
        let term = CoreTerm::SmtProof(cert);
        let ctx = Context::new();
        let reg = AxiomRegistry::new();
        let err = infer(&ctx, &term, &reg).unwrap_err();
        assert!(matches!(err, KernelError::EmptyCertificate));
    }

    #[test]
    fn smtproof_infer_rejects_future_schema() {
        let mut trace = List::new();
        trace.push(0x01);
        let mut cert = SmtCertificate::new(
            Text::from("cvc5"),
            Text::from("1.2.0"),
            trace,
            Text::from("sha256:1"),
        );
        cert.schema_version = CERTIFICATE_SCHEMA_VERSION + 5;
        let term = CoreTerm::SmtProof(cert);
        let ctx = Context::new();
        let reg = AxiomRegistry::new();
        let err = infer(&ctx, &term, &reg).unwrap_err();
        assert!(matches!(
            err,
            KernelError::UnsupportedCertificateSchema { .. }
        ));
    }

    #[test]
    fn smtproof_check_returns_bool_shape() {
        let mut trace = List::new();
        trace.push(0x01);
        let cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            trace,
            Text::from("sha256:feed"),
        );
        let term = CoreTerm::SmtProof(cert);
        let ctx = Context::new();
        let reg = AxiomRegistry::new();
        let head = check(&ctx, &term, &reg).unwrap();
        assert_eq!(head, CoreType::Inductive(Text::from("Bool")));
    }

    // -----------------------------------------------------------------
    // Envelope schema + metadata tests (task #75)
    // -----------------------------------------------------------------

    #[test]
    fn new_certificate_stamps_current_schema_version() {
        let cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            List::new(),
            Text::from("sha256:0"),
        );
        assert_eq!(cert.schema_version, CERTIFICATE_SCHEMA_VERSION);
        assert_eq!(cert.verum_version.as_str(), env!("CARGO_PKG_VERSION"));
        assert!(cert.metadata.is_empty());
        assert!(cert.created_at.as_str().is_empty());
    }

    #[test]
    fn legacy_unversioned_certificate_still_validates() {
        let mut cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            List::new(),
            Text::from("sha256:0"),
        );
        cert.schema_version = 0;
        assert!(cert.validate_schema().is_ok());
    }

    #[test]
    fn future_schema_version_is_rejected() {
        let mut cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            List::new(),
            Text::from("sha256:0"),
        );
        cert.schema_version = CERTIFICATE_SCHEMA_VERSION + 100;
        let err = cert.validate_schema().unwrap_err();
        assert!(matches!(
            err,
            KernelError::UnsupportedCertificateSchema { .. }
        ));
    }

    #[test]
    fn replay_rejects_future_schema_version() {
        let mut trace = List::new();
        trace.push(0x01);
        let mut cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            trace,
            Text::from("sha256:deadbeef"),
        );
        cert.schema_version = CERTIFICATE_SCHEMA_VERSION + 1;
        let err = replay_smt_cert(&Context::new(), &cert).unwrap_err();
        assert!(matches!(
            err,
            KernelError::UnsupportedCertificateSchema { found, max_supported }
                if found == CERTIFICATE_SCHEMA_VERSION + 1
                    && max_supported == CERTIFICATE_SCHEMA_VERSION
        ));
    }

    #[test]
    fn with_metadata_appends_keys_in_order() {
        let cert = SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.13.0"),
            List::new(),
            Text::from("sha256:0"),
        )
        .with_metadata(Text::from("tactic"), Text::from("omega"))
        .with_metadata(Text::from("duration_ms"), Text::from("42"))
        .with_created_at(Text::from("2026-04-23T12:34:56Z"));
        assert_eq!(cert.metadata.len(), 2);
        assert_eq!(cert.metadata[0].0.as_str(), "tactic");
        assert_eq!(cert.metadata[0].1.as_str(), "omega");
        assert_eq!(cert.metadata[1].0.as_str(), "duration_ms");
        assert_eq!(cert.created_at.as_str(), "2026-04-23T12:34:56Z");
    }

    #[test]
    fn serde_roundtrip_preserves_all_envelope_fields() {
        let cert = SmtCertificate::new(
            Text::from("cvc5"),
            Text::from("1.2.0"),
            {
                let mut t = List::new();
                t.push(0x03);
                t
            },
            Text::from("sha256:feed"),
        )
        .with_metadata(Text::from("solver_opts"), Text::from("--produce-proofs"))
        .with_created_at(Text::from("2026-04-23T00:00:00Z"));
        let json = serde_json::to_string(&cert).unwrap();
        let rehydrated: SmtCertificate = serde_json::from_str(&json).unwrap();
        assert_eq!(rehydrated, cert);
    }

    // -----------------------------------------------------------------
    // Dependent-type rules — Pi / Lam / App + substitution
    // -----------------------------------------------------------------

    /// `Type(0) : Type(1)`.
    #[test]
    fn universe_inhabits_successor() {
        let ctx = Context::new();
        let ax = AxiomRegistry::new();
        let ty = infer(
            &ctx,
            &CoreTerm::Universe(UniverseLevel::Concrete(0)),
            &ax,
        )
        .unwrap();
        assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(1)));
    }

    /// Polymorphic identity: `λ (A : Type) (x : A). x : (A : Type) → A → A`.
    ///
    /// This is the canonical smoke test for Π-introduction + App-elim
    /// with capture-avoiding substitution. `infer` must return the
    /// exact Π-type (not a shape-head abstraction) so App can destructure.
    #[test]
    fn polymorphic_identity_types_correctly() {
        let ax = AxiomRegistry::new();
        let id_lam = CoreTerm::Lam {
            binder: Text::from("A"),
            domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
            body: Heap::new(CoreTerm::Lam {
                binder: Text::from("x"),
                domain: Heap::new(CoreTerm::Var(Text::from("A"))),
                body: Heap::new(CoreTerm::Var(Text::from("x"))),
            }),
        };
        let ty = infer(&Context::new(), &id_lam, &ax).unwrap();
        // Expect: Pi (A : Type(0)) (Pi (x : A) A)
        assert!(matches!(
            ty,
            CoreTerm::Pi { ref binder, .. } if binder.as_str() == "A"
        ));
        let outer_codom = match ty {
            CoreTerm::Pi { codomain, .. } => codomain,
            _ => unreachable!(),
        };
        assert!(matches!(
            *outer_codom,
            CoreTerm::Pi { ref binder, .. } if binder.as_str() == "x"
        ));
    }

    /// `(λ (x : Unit). x) tt : Unit`  — App + beta-style substitution.
    #[test]
    fn application_of_identity_substitutes_argument() {
        let ax = AxiomRegistry::new();
        let tt = CoreTerm::Axiom {
            name: Text::from("tt"),
            ty: Heap::new(unit_ty()),
            framework: FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        };
        let mut reg = AxiomRegistry::new();
        reg.register(
            Text::from("tt"),
            unit_ty(),
            FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        )
        .unwrap();
        let _ = ax; // keep ax handle for compile cleanliness
        let id_lam = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(unit_ty()),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        };
        let applied = CoreTerm::App(Heap::new(id_lam), Heap::new(tt));
        let ty = infer(&Context::new(), &applied, &reg).unwrap();
        assert_eq!(ty, unit_ty());
    }

    /// App with domain mismatch is rejected.
    #[test]
    fn application_with_type_mismatch_rejects() {
        let mut reg = AxiomRegistry::new();
        // Register `zero` of type Nat, `tt` of type Unit.
        let nat_ty = CoreTerm::Inductive {
            path: Text::from("Nat"),
            args: List::new(),
        };
        reg.register(
            Text::from("zero"),
            nat_ty.clone(),
            FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("nat-introduction"),
            },
        )
        .unwrap();
        reg.register(
            Text::from("tt"),
            unit_ty(),
            FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        )
        .unwrap();

        // λ (x : Nat). x — identity over Nat
        let id_nat = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(nat_ty),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        };
        let tt = CoreTerm::Axiom {
            name: Text::from("tt"),
            ty: Heap::new(unit_ty()),
            framework: FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        };
        // (λ (x : Nat). x) tt  — tt is Unit, not Nat → must error.
        let applied = CoreTerm::App(Heap::new(id_nat), Heap::new(tt));
        let err = infer(&Context::new(), &applied, &reg).unwrap_err();
        assert!(matches!(err, KernelError::TypeMismatch { .. }));
    }

    /// Applying a non-function term produces NotAFunction.
    #[test]
    fn application_of_non_function_rejects() {
        let mut reg = AxiomRegistry::new();
        reg.register(
            Text::from("tt"),
            unit_ty(),
            FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        )
        .unwrap();
        let tt = CoreTerm::Axiom {
            name: Text::from("tt"),
            ty: Heap::new(unit_ty()),
            framework: FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("unit-introduction"),
            },
        };
        let applied = CoreTerm::App(Heap::new(tt.clone()), Heap::new(tt));
        let err = infer(&Context::new(), &applied, &reg).unwrap_err();
        assert!(matches!(err, KernelError::NotAFunction(_)));
    }

    /// Substitution does not cross a shadowing binder.
    #[test]
    fn substitute_stops_at_shadowing_binder() {
        let inner = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(unit_ty()),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        };
        let replaced = substitute(&inner, "x", &unit_ty());
        // The bound `x` inside the lambda must NOT be replaced.
        match &replaced {
            CoreTerm::Lam { body, .. } => {
                assert_eq!(**body, CoreTerm::Var(Text::from("x")));
            }
            _ => panic!("expected a lambda"),
        }
    }

    /// Substitution replaces free occurrences.
    #[test]
    fn substitute_replaces_free_occurrences() {
        let term = CoreTerm::App(
            Heap::new(CoreTerm::Var(Text::from("f"))),
            Heap::new(CoreTerm::Var(Text::from("a"))),
        );
        let replaced = substitute(&term, "a", &unit_ty());
        match replaced {
            CoreTerm::App(f, a) => {
                assert_eq!(*f, CoreTerm::Var(Text::from("f")));
                assert_eq!(*a, unit_ty());
            }
            _ => panic!("expected App"),
        }
    }

    /// Full-structural verify: identity lambda has the exact Π type.
    #[test]
    fn verify_full_accepts_matching_type() {
        let ax = AxiomRegistry::new();
        let id_unit = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(unit_ty()),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        };
        let expected = CoreTerm::Pi {
            binder: Text::from("x"),
            domain: Heap::new(unit_ty()),
            codomain: Heap::new(unit_ty()),
        };
        verify_full(&Context::new(), &id_unit, &expected, &ax).unwrap();
    }

    #[test]
    fn verify_full_rejects_mismatched_type() {
        let ax = AxiomRegistry::new();
        let id_unit = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(unit_ty()),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        };
        let wrong = CoreTerm::Universe(UniverseLevel::Concrete(0));
        let err =
            verify_full(&Context::new(), &id_unit, &wrong, &ax).unwrap_err();
        assert!(matches!(err, KernelError::TypeMismatch { .. }));
    }

    // -----------------------------------------------------------------
    // Σ-type rules — Sigma / Pair / Fst / Snd
    // -----------------------------------------------------------------

    fn tt_axiom(reg: &mut AxiomRegistry) -> CoreTerm {
        let fw = FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("unit-introduction"),
        };
        let _ = reg.register(Text::from("tt"), unit_ty(), fw.clone());
        CoreTerm::Axiom {
            name: Text::from("tt"),
            ty: Heap::new(unit_ty()),
            framework: fw,
        }
    }

    /// Σ-formation: `Sigma (x : Unit) Unit` is a type.
    #[test]
    fn sigma_formation_returns_universe() {
        let ax = AxiomRegistry::new();
        let sigma = CoreTerm::Sigma {
            binder: Text::from("x"),
            fst_ty: Heap::new(unit_ty()),
            snd_ty: Heap::new(unit_ty()),
        };
        let ty = infer(&Context::new(), &sigma, &ax).unwrap();
        assert!(matches!(ty, CoreTerm::Universe(_)));
    }

    /// Non-dependent Pair: (tt, tt) : Sigma (_:Unit) Unit.
    #[test]
    fn pair_introduction_builds_sigma() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let pair = CoreTerm::Pair(Heap::new(tt.clone()), Heap::new(tt));
        let ty = infer(&Context::new(), &pair, &reg).unwrap();
        match ty {
            CoreTerm::Sigma { fst_ty, snd_ty, .. } => {
                assert_eq!(*fst_ty, unit_ty());
                assert_eq!(*snd_ty, unit_ty());
            }
            _ => panic!("expected Sigma"),
        }
    }

    /// `fst((tt, tt)) : Unit`.
    #[test]
    fn fst_projection_types_to_first_component() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let pair = CoreTerm::Pair(Heap::new(tt.clone()), Heap::new(tt));
        let fst = CoreTerm::Fst(Heap::new(pair));
        let ty = infer(&Context::new(), &fst, &reg).unwrap();
        assert_eq!(ty, unit_ty());
    }

    /// `snd((tt, tt)) : Unit` — since the Σ-type is non-dependent,
    /// substitution doesn't change anything and we still get Unit.
    #[test]
    fn snd_projection_types_to_second_component() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let pair = CoreTerm::Pair(Heap::new(tt.clone()), Heap::new(tt));
        let snd = CoreTerm::Snd(Heap::new(pair));
        let ty = infer(&Context::new(), &snd, &reg).unwrap();
        assert_eq!(ty, unit_ty());
    }

    /// `fst(tt)` — tt : Unit is not a pair — rejected.
    #[test]
    fn fst_of_non_pair_rejects() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let wrong = CoreTerm::Fst(Heap::new(tt));
        let err = infer(&Context::new(), &wrong, &reg).unwrap_err();
        assert!(matches!(err, KernelError::NotAPair(_)));
    }

    // -----------------------------------------------------------------
    // Cubical Path-type rules — PathTy / Refl
    // -----------------------------------------------------------------

    /// `Path<Unit>(tt, tt) : Type(0)`.
    #[test]
    fn path_formation_returns_carrier_universe() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let path = CoreTerm::PathTy {
            carrier: Heap::new(unit_ty()),
            lhs: Heap::new(tt.clone()),
            rhs: Heap::new(tt),
        };
        let ty = infer(&Context::new(), &path, &reg).unwrap();
        assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
    }

    /// `refl(tt) : Path<Unit>(tt, tt)`.
    #[test]
    fn refl_produces_path_type_with_identical_endpoints() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let refl = CoreTerm::Refl(Heap::new(tt.clone()));
        let ty = infer(&Context::new(), &refl, &reg).unwrap();
        match ty {
            CoreTerm::PathTy { carrier, lhs, rhs } => {
                assert_eq!(*carrier, unit_ty());
                assert_eq!(*lhs, tt);
                assert_eq!(*rhs, tt);
            }
            _ => panic!("expected PathTy"),
        }
    }

    // -----------------------------------------------------------------
    // Refinement / cubical / Elim — bring-up rules
    // -----------------------------------------------------------------

    /// `{x : Unit | tt} : Type(0)` — refinement formation.
    #[test]
    fn refine_formation_returns_base_universe() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let refined = CoreTerm::Refine {
            base: Heap::new(unit_ty()),
            binder: Text::from("x"),
            predicate: Heap::new(tt),
        };
        let ty = infer(&Context::new(), &refined, &reg).unwrap();
        assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
    }

    /// `hcomp φ walls tt : Unit`. The strengthened rule requires
    /// phi/walls to be well-typed; bind them in the context so
    /// inference succeeds.
    #[test]
    fn hcomp_infers_base_type() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let ctx = Context::new()
            .extend(Text::from("phi"), unit_ty())
            .extend(Text::from("walls"), unit_ty());
        let hc = CoreTerm::HComp {
            phi: Heap::new(CoreTerm::Var(Text::from("phi"))),
            walls: Heap::new(CoreTerm::Var(Text::from("walls"))),
            base: Heap::new(tt),
        };
        let ty = infer(&ctx, &hc, &reg).unwrap();
        assert_eq!(ty, unit_ty());
    }

    /// HComp with an ill-typed `phi` is rejected — the strengthened
    /// rule no longer swallows subterm errors.
    #[test]
    fn hcomp_rejects_unbound_phi() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let hc = CoreTerm::HComp {
            phi: Heap::new(CoreTerm::Var(Text::from("phi_unbound"))),
            walls: Heap::new(tt.clone()),
            base: Heap::new(tt),
        };
        let err = infer(&Context::new(), &hc, &reg).unwrap_err();
        assert!(matches!(err, KernelError::UnboundVariable(_)));
    }

    /// `transp path r tt` — returns the path's right endpoint type.
    #[test]
    fn transp_returns_path_rhs_type() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let ctx = Context::new().extend(Text::from("r"), unit_ty());
        let path = CoreTerm::PathTy {
            carrier: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
            lhs: Heap::new(unit_ty()),
            rhs: Heap::new(unit_ty()),
        };
        let tr = CoreTerm::Transp {
            path: Heap::new(path),
            regular: Heap::new(CoreTerm::Var(Text::from("r"))),
            value: Heap::new(tt),
        };
        let ty = infer(&ctx, &tr, &reg).unwrap();
        assert_eq!(ty, unit_ty());
    }

    /// Transp with an unbound regular endpoint is rejected.
    #[test]
    fn transp_rejects_unbound_regular() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        let path = CoreTerm::PathTy {
            carrier: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
            lhs: Heap::new(unit_ty()),
            rhs: Heap::new(unit_ty()),
        };
        let tr = CoreTerm::Transp {
            path: Heap::new(path),
            regular: Heap::new(CoreTerm::Var(Text::from("r_unbound"))),
            value: Heap::new(tt),
        };
        let err = infer(&Context::new(), &tr, &reg).unwrap_err();
        assert!(matches!(err, KernelError::UnboundVariable(_)));
    }

    /// `Glue<Unit>(…)` inhabits the universe of its carrier.
    #[test]
    fn glue_returns_carrier_universe() {
        let ax = AxiomRegistry::new();
        let ctx = Context::new()
            .extend(Text::from("phi"), unit_ty())
            .extend(Text::from("fiber"), unit_ty())
            .extend(Text::from("equiv"), unit_ty());
        let g = CoreTerm::Glue {
            carrier: Heap::new(unit_ty()),
            phi: Heap::new(CoreTerm::Var(Text::from("phi"))),
            fiber: Heap::new(CoreTerm::Var(Text::from("fiber"))),
            equiv: Heap::new(CoreTerm::Var(Text::from("equiv"))),
        };
        let ty = infer(&ctx, &g, &ax).unwrap();
        assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
    }

    /// Glue whose equiv is ill-typed — rejected.
    #[test]
    fn glue_rejects_unbound_equiv() {
        let ax = AxiomRegistry::new();
        let ctx = Context::new()
            .extend(Text::from("phi"), unit_ty())
            .extend(Text::from("fiber"), unit_ty());
        let g = CoreTerm::Glue {
            carrier: Heap::new(unit_ty()),
            phi: Heap::new(CoreTerm::Var(Text::from("phi"))),
            fiber: Heap::new(CoreTerm::Var(Text::from("fiber"))),
            equiv: Heap::new(CoreTerm::Var(Text::from("equiv_unbound"))),
        };
        let err = infer(&ctx, &g, &ax).unwrap_err();
        assert!(matches!(err, KernelError::UnboundVariable(_)));
    }

    /// Glue whose carrier is not in a universe is rejected.
    #[test]
    fn glue_rejects_non_universe_carrier() {
        let mut reg = AxiomRegistry::new();
        // tt is of type Unit, which is not a universe.
        let tt = tt_axiom(&mut reg);
        let ctx = Context::new()
            .extend(Text::from("phi"), unit_ty())
            .extend(Text::from("fiber"), unit_ty())
            .extend(Text::from("equiv"), unit_ty());
        let g = CoreTerm::Glue {
            // Glue expects carrier to inhabit a universe. `tt : Unit`
            // inhabits Unit (a type), not a universe, so
            // `universe_level` rejects it.
            carrier: Heap::new(tt),
            phi: Heap::new(CoreTerm::Var(Text::from("phi"))),
            fiber: Heap::new(CoreTerm::Var(Text::from("fiber"))),
            equiv: Heap::new(CoreTerm::Var(Text::from("equiv"))),
        };
        // This should fail because tt's type (Unit) is not a universe.
        let res = infer(&ctx, &g, &reg);
        assert!(res.is_err());
    }

    /// `elim e motive cases : motive e` — shape-level Elim rule.
    #[test]
    fn elim_types_to_motive_applied_to_scrutinee() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        // motive : Unit → Type(0) — represented as λ(u:Unit). Unit
        let motive = CoreTerm::Lam {
            binder: Text::from("u"),
            domain: Heap::new(unit_ty()),
            body: Heap::new(unit_ty()),
        };
        let e = CoreTerm::Elim {
            scrutinee: Heap::new(tt.clone()),
            motive: Heap::new(motive.clone()),
            cases: List::new(),
        };
        let ty = infer(&Context::new(), &e, &reg).unwrap();
        match ty {
            CoreTerm::App(f, a) => {
                assert_eq!(*f, motive);
                assert_eq!(*a, tt);
            }
            _ => panic!("expected App (motive scrutinee)"),
        }
    }

    /// `Path<Unit>(tt, someBool)` — endpoint type mismatch — rejected.
    /// Demonstrates that the kernel checks endpoint types against the
    /// declared carrier, not just shape.
    #[test]
    fn path_rejects_endpoint_type_mismatch() {
        let mut reg = AxiomRegistry::new();
        let tt = tt_axiom(&mut reg);
        // Register an axiom of a *different* type.
        let bool_ty = CoreTerm::Inductive {
            path: Text::from("Bool"),
            args: List::new(),
        };
        reg.register(
            Text::from("true_val"),
            bool_ty.clone(),
            FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("bool-introduction"),
            },
        )
        .unwrap();
        let true_val = CoreTerm::Axiom {
            name: Text::from("true_val"),
            ty: Heap::new(bool_ty),
            framework: FrameworkId {
                framework: Text::from("builtin"),
                citation: Text::from("bool-introduction"),
            },
        };
        let bogus = CoreTerm::PathTy {
            carrier: Heap::new(unit_ty()),
            lhs: Heap::new(tt),
            rhs: Heap::new(true_val),
        };
        let err = infer(&Context::new(), &bogus, &reg).unwrap_err();
        assert!(matches!(err, KernelError::TypeMismatch { .. }));
    }

    // -----------------------------------------------------------------
    // FrameworkAttr → AxiomRegistry loader
    // -----------------------------------------------------------------

    /// Helper: build a parsed module with one `@framework(id, "cite")`
    /// axiom declaration.
    fn module_with_axiom(
        framework_name: &str,
        citation: &str,
        axiom_name: &str,
    ) -> verum_ast::Module {
        use verum_ast::attr::Attribute;
        use verum_ast::decl::{AxiomDecl, Visibility};
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::literal::{Literal, LiteralKind, StringLit};
        use verum_ast::span::Span;
        use verum_ast::{Ident, Item, ItemKind};

        let span = Span::default();
        let name_expr = Expr::ident(Ident::new(Text::from(framework_name), span));
        let cite_lit = Literal::new(
            LiteralKind::Text(StringLit::Regular(Text::from(citation))),
            span,
        );
        let cite_expr = Expr::literal(cite_lit);

        let mut args: List<Expr> = List::new();
        args.push(name_expr);
        args.push(cite_expr);
        let framework_attr =
            Attribute::new(Text::from("framework"), Maybe::Some(args), span);

        let mut attrs: List<Attribute> = List::new();
        attrs.push(framework_attr);

        // Minimal AxiomDecl — body and clauses stay empty; the
        // loader only inspects name + attributes.
        let axiom_ident = Ident::new(Text::from(axiom_name), span);
        let proposition = Expr::literal(Literal::new(
            LiteralKind::Bool(true),
            span,
        ));
        let decl = AxiomDecl::new(axiom_ident, proposition, span);
        let mut decl = decl;
        decl.visibility = Visibility::Public;
        decl.attributes = attrs.clone();

        let item = Item {
            kind: ItemKind::Axiom(decl),
            attributes: List::new(),
            span,
        };

        let mut items: List<Item> = List::new();
        items.push(item);
        verum_ast::Module {
            items,
            span,
            file_id: verum_ast::span::FileId::new(0),
            attributes: List::new(),
        }
    }

    #[test]
    fn load_framework_axioms_registers_single_marker() {
        let module = module_with_axiom(
            "lurie_htt",
            "HTT 6.2.2.7",
            "sheafification_is_topos",
        );
        let mut reg = AxiomRegistry::new();
        let report = load_framework_axioms(&module, &mut reg);

        assert!(report.is_clean(), "expected clean load, got {:?}", report);
        assert_eq!(report.registered.len(), 1);
        assert_eq!(
            report.registered.get(0).map(|t| t.as_str()),
            Some("sheafification_is_topos")
        );

        match reg.get("sheafification_is_topos") {
            Maybe::Some(entry) => {
                assert_eq!(entry.framework.framework.as_str(), "lurie_htt");
                assert_eq!(entry.framework.citation.as_str(), "HTT 6.2.2.7");
            }
            Maybe::None => panic!("axiom not registered"),
        }
    }

    #[test]
    fn load_framework_axioms_detects_duplicate() {
        let m1 = module_with_axiom(
            "lurie_htt",
            "HTT 6.2.2.7",
            "sheafification_is_topos",
        );
        let m2 = module_with_axiom(
            "schreiber_dcct",
            "DCCT §3.9",
            "sheafification_is_topos", // same name — collision
        );
        let mut reg = AxiomRegistry::new();
        let r1 = load_framework_axioms(&m1, &mut reg);
        assert!(r1.is_clean());

        let r2 = load_framework_axioms(&m2, &mut reg);
        assert_eq!(r2.duplicates.len(), 1);
        assert_eq!(
            r2.duplicates.get(0).map(|t| t.as_str()),
            Some("sheafification_is_topos")
        );
        assert!(r2.registered.is_empty());
    }

    #[test]
    fn load_framework_axioms_skips_non_axiom_items() {
        // A theorem with @framework is NOT auto-registered — the
        // loader only consumes axioms. (Theorems are consumers, not
        // postulates, so their elaborator path handles registration
        // when a proof term is submitted.)
        use verum_ast::attr::Attribute;
        use verum_ast::decl::{TheoremDecl, Visibility};
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::literal::{Literal, LiteralKind, StringLit};
        use verum_ast::span::Span;
        use verum_ast::{Ident, Item, ItemKind};

        let span = Span::default();

        let framework_attr = {
            let name_expr = Expr::ident(Ident::new(Text::from("lurie_htt"), span));
            let cite_lit = Literal::new(
                LiteralKind::Text(StringLit::Regular(Text::from("HTT 6.2.2.7"))),
                span,
            );
            let cite_expr = Expr::literal(cite_lit);
            let mut args: List<Expr> = List::new();
            args.push(name_expr);
            args.push(cite_expr);
            Attribute::new(Text::from("framework"), Maybe::Some(args), span)
        };
        let mut attrs: List<Attribute> = List::new();
        attrs.push(framework_attr);

        let theorem_ident = Ident::new(Text::from("some_theorem"), span);
        let mut thm = TheoremDecl::new(
            theorem_ident,
            Expr::literal(Literal::new(LiteralKind::Bool(true), span)),
            span,
        );
        thm.visibility = Visibility::Public;
        thm.attributes = attrs;

        let item = Item {
            kind: ItemKind::Theorem(thm),
            attributes: List::new(),
            span,
        };

        let mut items: List<Item> = List::new();
        items.push(item);
        let module = verum_ast::Module {
            items,
            span,
            file_id: verum_ast::span::FileId::new(0),
            attributes: List::new(),
        };

        let mut reg = AxiomRegistry::new();
        let report = load_framework_axioms(&module, &mut reg);
        assert!(report.is_clean());
        assert!(report.registered.is_empty());
        assert_eq!(reg.all().len(), 0);
    }

    #[test]
    fn axiom_after_load_is_checkable_by_infer() {
        // End-to-end: load a framework axiom, then successfully
        // check a CoreTerm::Axiom that references it.
        let module = module_with_axiom(
            "connes_reconstruction",
            "Connes 2008 axiom (vii)",
            "first_order_condition",
        );
        let mut reg = AxiomRegistry::new();
        let report = load_framework_axioms(&module, &mut reg);
        assert!(report.is_clean());

        let term = CoreTerm::Axiom {
            name: Text::from("first_order_condition"),
            ty: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
            framework: FrameworkId {
                framework: Text::from("connes_reconstruction"),
                citation: Text::from("Connes 2008 axiom (vii)"),
            },
        };
        let ty = infer(&Context::new(), &term, &reg).unwrap();
        assert!(matches!(ty, CoreTerm::Universe(_)));
    }

    // -------------------------------------------------------------
    // Rule 10: UIP-Free — axioms reducing to UIP are rejected.
    // -------------------------------------------------------------

    /// Build the direct UIP statement:
    /// `Π A. Π a. Π b. Π p. Π q. PathTy(PathTy(A, a, b), p, q)`.
    fn uip_statement() -> CoreTerm {
        fn var(n: &str) -> CoreTerm {
            CoreTerm::Var(Text::from(n))
        }
        let path_a_a_b = CoreTerm::PathTy {
            carrier: Heap::new(var("A")),
            lhs: Heap::new(var("a")),
            rhs: Heap::new(var("b")),
        };
        let path_of_paths = CoreTerm::PathTy {
            carrier: Heap::new(path_a_a_b.clone()),
            lhs: Heap::new(var("p")),
            rhs: Heap::new(var("q")),
        };
        let pi_q = CoreTerm::Pi {
            binder: Text::from("q"),
            domain: Heap::new(path_a_a_b.clone()),
            codomain: Heap::new(path_of_paths),
        };
        let pi_p = CoreTerm::Pi {
            binder: Text::from("p"),
            domain: Heap::new(path_a_a_b),
            codomain: Heap::new(pi_q),
        };
        let pi_b = CoreTerm::Pi {
            binder: Text::from("b"),
            domain: Heap::new(var("A")),
            codomain: Heap::new(pi_p),
        };
        let pi_a_val = CoreTerm::Pi {
            binder: Text::from("a"),
            domain: Heap::new(var("A")),
            codomain: Heap::new(pi_b),
        };
        CoreTerm::Pi {
            binder: Text::from("A"),
            domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
            codomain: Heap::new(pi_a_val),
        }
    }

    #[test]
    fn uip_axiom_is_rejected_by_register() {
        let mut reg = AxiomRegistry::new();
        let result = reg.register(
            Text::from("uip"),
            uip_statement(),
            FrameworkId {
                framework: Text::from("set_level"),
                citation: Text::from("attempted UIP postulate"),
            },
        );
        match result {
            Err(KernelError::UipForbidden(name)) => {
                assert_eq!(name.as_str(), "uip");
            }
            other => panic!("expected UipForbidden, got {:?}", other),
        }
        assert_eq!(reg.all().len(), 0);
    }

    #[test]
    fn non_uip_axiom_is_accepted() {
        // A plain axiom claiming a proposition about a universe
        // is not UIP and must not be rejected by rule 10.
        let mut reg = AxiomRegistry::new();
        let result = reg.register(
            Text::from("some_axiom"),
            CoreTerm::Universe(UniverseLevel::Concrete(0)),
            FrameworkId {
                framework: Text::from("test"),
                citation: Text::from("test"),
            },
        );
        assert!(result.is_ok());
        assert_eq!(reg.all().len(), 1);
    }

    #[test]
    fn single_pi_over_path_is_not_uip() {
        // A single Π over a path type is NOT UIP — guard should not
        // false-positive on partial shapes.
        fn var(n: &str) -> CoreTerm {
            CoreTerm::Var(Text::from(n))
        }
        let almost = CoreTerm::Pi {
            binder: Text::from("A"),
            domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
            codomain: Heap::new(CoreTerm::PathTy {
                carrier: Heap::new(var("A")),
                lhs: Heap::new(var("a")),
                rhs: Heap::new(var("b")),
            }),
        };
        let mut reg = AxiomRegistry::new();
        let result = reg.register(
            Text::from("path_forall"),
            almost,
            FrameworkId {
                framework: Text::from("test"),
                citation: Text::from("test"),
            },
        );
        assert!(result.is_ok(), "partial shape must not trigger UIP guard");
    }
}
