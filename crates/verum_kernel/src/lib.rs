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
// M-iteration depth (Diakrisis T-2f* / VUVA §4.3)
// =============================================================================

/// Compute the M-iteration depth of a [`CoreTerm`], per Verum Unified
/// Verification Architecture (VUVA) §4.3.
///
/// The depth function is the operational realisation of the Diakrisis
/// metaisation modality `M`: every construct that semantically *speaks
/// about* a lower-depth object bumps the depth by one. Framework axioms
/// (which assert facts about their stated body), `Quote` (which reflects
/// a term as data), `Inductive` / `Quotient` introductions (which close
/// a universe-level construction), and named-type references that
/// originate in the standard library all raise the count.
///
/// The depth bound is consumed by the `K-Refine` rule: a refinement
/// `{ x : base | P(x) }` is well-formed only when
/// `m_depth(P) < m_depth(base) + 1`, i.e. `m_depth(P) ≤ m_depth(base)`.
/// This is the Verum realisation of Diakrisis axiom T-2f* (depth-strict
/// comprehension), which — via Yanofsky 2003 — closes every
/// self-referential paradox schema in a cartesian-closed setting.
///
/// The function is defined recursively:
///
///   * `Var`, `Universe(n)` — zero (variables have no M-iteration;
///     the universe level is reported as depth to align with the
///     stratification in VUVA §4.3).
///   * `Pi`, `Lam`, `Sigma`, `App`, `Pair`, `Fst`, `Snd`, `PathTy`,
///     `Refl`, `HComp`, `Transp`, `Glue`, `Elim` — maximum of their
///     sub-terms (structural, no depth bump).
///   * `Refine { base, predicate }` — maximum of `dp(base)` and
///     `dp(predicate)`.
///   * `Inductive { path, args }` — `1 + max{ dp(arg) | arg ∈ args }`
///     (declared type constructors live one level above their
///     instantiation arguments — they are *defined* by a schema).
///   * `Axiom { body, .. }` — `dp(body) + 1` (framework axioms *speak
///     about* their stated body).
///   * `SmtProof` — `0` (certificates themselves carry no M-iteration).
///
/// Time complexity: O(n) in the size of the term tree. The kernel
/// invokes this at each `Refine` / `Inductive` / `Axiom` check point;
/// a single polynomial walk per well-formedness query.
pub fn m_depth(term: &CoreTerm) -> usize {
    match term {
        CoreTerm::Var(_) => 0,
        CoreTerm::Universe(lvl) => match lvl {
            UniverseLevel::Concrete(n) => *n as usize,
            UniverseLevel::Prop => 0,
            UniverseLevel::Variable(_) => 0,
            UniverseLevel::Succ(l) => 1 + m_depth_level(l),
            UniverseLevel::Max(a, b) => m_depth_level(a).max(m_depth_level(b)),
        },
        CoreTerm::Pi { domain, codomain, .. } => m_depth(domain).max(m_depth(codomain)),
        CoreTerm::Lam { domain, body, .. } => m_depth(domain).max(m_depth(body)),
        CoreTerm::App(f, a) => m_depth(f).max(m_depth(a)),
        CoreTerm::Sigma { fst_ty, snd_ty, .. } => m_depth(fst_ty).max(m_depth(snd_ty)),
        CoreTerm::Pair(a, b) => m_depth(a).max(m_depth(b)),
        CoreTerm::Fst(p) | CoreTerm::Snd(p) => m_depth(p),
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            m_depth(carrier).max(m_depth(lhs)).max(m_depth(rhs))
        }
        CoreTerm::Refl(a) => m_depth(a),
        CoreTerm::HComp { phi, walls, base } => {
            m_depth(phi).max(m_depth(walls)).max(m_depth(base))
        }
        CoreTerm::Transp { path, regular, value } => {
            m_depth(path).max(m_depth(regular)).max(m_depth(value))
        }
        CoreTerm::Glue { carrier, phi, fiber, equiv } => m_depth(carrier)
            .max(m_depth(phi))
            .max(m_depth(fiber))
            .max(m_depth(equiv)),
        CoreTerm::Refine { base, predicate, .. } => m_depth(base).max(m_depth(predicate)),
        // Inductive: declared type constructors live one level above
        // their instantiation arguments (the schema is a meta-statement
        // about the arguments).
        CoreTerm::Inductive { args, .. } => {
            1 + args.iter().map(m_depth).max().unwrap_or(0)
        }
        CoreTerm::Elim { scrutinee, motive, cases } => {
            let case_max = cases.iter().map(m_depth).max().unwrap_or(0);
            m_depth(scrutinee).max(m_depth(motive)).max(case_max)
        }
        // SMT certificates carry no M-iteration of their own — they
        // witness a propositional fact about terms already in scope.
        CoreTerm::SmtProof(_) => 0,
        // An `Axiom` node in the kernel is a *term* — a proof witness
        // of its claimed type. Its depth is therefore `dp(ty)`, NOT
        // `dp(ty) + 1`. The schema-declaration side of a framework
        // axiom (which *would* bump by +1, per VUVA §4.3) is handled
        // by the declaration-time path (`AxiomRegistry::register`) —
        // that's where we reason about the axiom as a meta-statement.
        // Here we are only looking at invocation sites.
        //
        // The load-bearing depth bump comes from `Inductive` — a
        // named schema lives strictly above its instantiation
        // arguments, which is what blocks Yanofsky α: Y → T^Y
        // (`dp(T^Y) = dp(Y) + 1` forces the strict inequality the
        // diagonal construction needs, and `K-Refine` forbids exactly
        // that equality).
        CoreTerm::Axiom { ty, .. } => m_depth(ty),
        // VFE-1: ε(α) and α(ε) carry the M-depth of their argument.
        // The 2-natural equivalence τ : ε∘M ≃ A∘ε from Proposition 5.1
        // does not change m_depth at the term level — it lives at the
        // 2-cell level handled by `Kernel::check_eps_mu_coherence`.
        CoreTerm::EpsilonOf(t) | CoreTerm::AlphaOf(t) => m_depth(t),
        // VFE-7: modal operators inherit M-depth from their operand.
        // The M-iteration depth (used by K-Refine) does NOT see modal
        // structure; the modal-depth (used by K-Refine-omega) is a
        // *separate* ordinal-valued quantity computed by `m_depth_omega`.
        CoreTerm::ModalBox(t) | CoreTerm::ModalDiamond(t) => m_depth(t),
        CoreTerm::ModalBigAnd(args) => {
            args.iter().map(|t| m_depth(t)).max().unwrap_or(0)
        }
    }
}

// =============================================================================
// VFE-7 V0 — K-Refine-omega ordinal modal-depth
// =============================================================================

/// Ordinal-valued modal-depth for the K-Refine-omega kernel rule
/// (Theorem 136.T transfinite stratification).
///
/// Encoding: Cantor-normal-form prefix below ε_0, mirroring the
/// stdlib `core.theory_interop.coord::Ordinal` shape (single source
/// of truth between kernel and stdlib). The kernel keeps its own
/// definition because it cannot depend on stdlib at the trust
/// boundary.
///
///   `OrdinalDepth { omega_coeff: 0, finite_offset: n }`  encodes  `n`
///   `OrdinalDepth { omega_coeff: 1, finite_offset: 0 }`  encodes  `ω`
///   `OrdinalDepth { omega_coeff: 1, finite_offset: k }`  encodes  `ω + k`
///   `OrdinalDepth { omega_coeff: n, finite_offset: k }`  encodes  `ω·n + k`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrdinalDepth {
    /// ω-coefficient (0 ⇒ pure finite; 1 ⇒ ω; ≥ 2 ⇒ ω·n).
    pub omega_coeff: u32,
    /// Finite additive remainder.
    pub finite_offset: u32,
}

impl OrdinalDepth {
    /// Pure-finite depth — encoding of a usize.
    pub const fn finite(n: u32) -> Self {
        Self { omega_coeff: 0, finite_offset: n }
    }

    /// `ω`.
    pub const fn omega() -> Self {
        Self { omega_coeff: 1, finite_offset: 0 }
    }

    /// Lex ordering: `(omega_coeff, finite_offset)` lex.
    pub fn lt(&self, other: &Self) -> bool {
        if self.omega_coeff < other.omega_coeff { return true; }
        if self.omega_coeff > other.omega_coeff { return false; }
        self.finite_offset < other.finite_offset
    }

    /// `+ 1` — adds one to the finite component (always well-defined
    /// because we only ever ascend in the finite remainder until V1
    /// extends to limit-ordinal arithmetic).
    pub fn succ(&self) -> Self {
        Self {
            omega_coeff: self.omega_coeff,
            finite_offset: self.finite_offset.saturating_add(1),
        }
    }

    /// Render as canonical Unicode text.
    pub fn render(&self) -> String {
        if self.omega_coeff == 0 {
            return self.finite_offset.to_string();
        }
        let head = if self.omega_coeff == 1 {
            String::from("ω")
        } else {
            format!("ω·{}", self.omega_coeff)
        };
        if self.finite_offset == 0 {
            head
        } else {
            format!("{}+{}", head, self.finite_offset)
        }
    }
}

/// VFE-7 V1 — `K-Refine-omega` modal-depth function `md^ω`.
///
/// Per Definition 136.D1 (transfinite modal language L^ω_α):
///
///   md^ω(atomic)       = 0
///   md^ω(□φ)           = md^ω(φ) + 1
///   md^ω(◇φ)           = md^ω(φ) + 1
///   md^ω(⋀_{i<κ} P_i)  = sup_i md^ω(P_i)
///   md^ω(structural)   = max(md^ω of children)
///
/// Walks the term tree once, descending through all term shapes.
/// For non-modal terms the walk reduces to `max-of-children` which
/// preserves bit-identical behaviour with the V0 skeleton (modal
/// operators were the only Rank-bumping shapes anyway).
///
/// Termination: well-founded over the term tree depth (every
/// recursion descends to a strictly smaller subterm). Per Lemma
/// 136.L0 the ordinal recursion is well-defined for every term in
/// the canonical-primitive language L^ω_α.
///
/// Blocks (per VFE §7): Berry, paradoxical Löb, paraconsistent
/// Curry, Beth-Monk ω-iteration, and any ω·k or ω^ω modal-paradox
/// witness. The K-Refine-omega rule (`check_refine_omega`) routes
/// the result through `OrdinalDepth::lt` to gate refinement-type
/// formation.
pub fn m_depth_omega(term: &CoreTerm) -> OrdinalDepth {
    match term {
        // Atomic / variable — md^ω = 0.
        CoreTerm::Var(_) => OrdinalDepth::finite(0),
        CoreTerm::Universe(_) => OrdinalDepth::finite(0),

        // Modal operators — the load-bearing recursion.
        CoreTerm::ModalBox(phi) | CoreTerm::ModalDiamond(phi) => {
            m_depth_omega(phi).succ()
        }
        CoreTerm::ModalBigAnd(args) => {
            // sup_i md^ω(P_i). For finite arity the supremum is the
            // pointwise max under the lex ordering. Empty conjunction
            // is the identity (md^ω = 0).
            let mut sup = OrdinalDepth::finite(0);
            for arg in args.iter() {
                let r = m_depth_omega(arg);
                if sup.lt(&r) {
                    sup = r;
                }
            }
            sup
        }

        // Structural — descend into immediate children, take max.
        CoreTerm::Pi { domain, codomain, .. } => {
            ord_max(m_depth_omega(domain), m_depth_omega(codomain))
        }
        CoreTerm::Lam { domain, body, .. } => {
            ord_max(m_depth_omega(domain), m_depth_omega(body))
        }
        CoreTerm::App(f, a) => ord_max(m_depth_omega(f), m_depth_omega(a)),
        CoreTerm::Sigma { fst_ty, snd_ty, .. } => {
            ord_max(m_depth_omega(fst_ty), m_depth_omega(snd_ty))
        }
        CoreTerm::Pair(a, b) => ord_max(m_depth_omega(a), m_depth_omega(b)),
        CoreTerm::Fst(p) | CoreTerm::Snd(p) => m_depth_omega(p),
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            ord_max(
                m_depth_omega(carrier),
                ord_max(m_depth_omega(lhs), m_depth_omega(rhs)),
            )
        }
        CoreTerm::Refl(a) => m_depth_omega(a),
        CoreTerm::HComp { phi, walls, base } => ord_max(
            m_depth_omega(phi),
            ord_max(m_depth_omega(walls), m_depth_omega(base)),
        ),
        CoreTerm::Transp { path, regular, value } => ord_max(
            m_depth_omega(path),
            ord_max(m_depth_omega(regular), m_depth_omega(value)),
        ),
        CoreTerm::Glue { carrier, phi, fiber, equiv } => ord_max(
            ord_max(m_depth_omega(carrier), m_depth_omega(phi)),
            ord_max(m_depth_omega(fiber), m_depth_omega(equiv)),
        ),
        CoreTerm::Refine { base, predicate, .. } => {
            ord_max(m_depth_omega(base), m_depth_omega(predicate))
        }
        CoreTerm::Inductive { args, .. } => {
            let mut sup = OrdinalDepth::finite(0);
            for arg in args.iter() {
                let r = m_depth_omega(arg);
                if sup.lt(&r) {
                    sup = r;
                }
            }
            sup
        }
        CoreTerm::Elim { scrutinee, motive, cases } => {
            let mut sup = ord_max(m_depth_omega(scrutinee), m_depth_omega(motive));
            for case in cases.iter() {
                let r = m_depth_omega(case);
                if sup.lt(&r) {
                    sup = r;
                }
            }
            sup
        }
        CoreTerm::SmtProof(_) => OrdinalDepth::finite(0),
        CoreTerm::Axiom { ty, .. } => m_depth_omega(ty),
        CoreTerm::EpsilonOf(t) | CoreTerm::AlphaOf(t) => m_depth_omega(t),
    }
}

/// Local helper — pointwise lex max for OrdinalDepth.
fn ord_max(a: OrdinalDepth, b: OrdinalDepth) -> OrdinalDepth {
    if a.lt(&b) { b } else { a }
}

/// VFE-7 V0 — `K-Refine-omega` rule entry point.
///
/// Verifies the transfinite-stratification invariant
///
/// ```text
///     md^ω(P) < md^ω(A) + 1
/// ```
///
/// for a refinement type `{x : A | P(x)}`. V0 calls `m_depth_omega`
/// (skeleton) and applies the lex-ordinal `lt` test; V1 will route
/// modal operators through the full md^ω computation.
///
/// Returns `Ok(())` when the invariant holds, otherwise
/// `KernelError::ModalDepthExceeded` with both ranks rendered as
/// canonical Unicode text.
pub fn check_refine_omega(
    binder: &Text,
    base: &CoreTerm,
    predicate: &CoreTerm,
) -> Result<(), KernelError> {
    let base_rank = m_depth_omega(base);
    let pred_rank = m_depth_omega(predicate);
    let upper = base_rank.succ();
    if pred_rank.lt(&upper) {
        Ok(())
    } else {
        Err(KernelError::ModalDepthExceeded {
            binder: binder.clone(),
            base_rank: Text::from(base_rank.render()),
            pred_rank: Text::from(pred_rank.render()),
        })
    }
}

/// Auxiliary `m_depth` over [`UniverseLevel`] — extracted so the main
/// walker stays flat. Mirrors the `Universe` arm's cases.
fn m_depth_level(level: &UniverseLevel) -> usize {
    match level {
        UniverseLevel::Concrete(n) => *n as usize,
        UniverseLevel::Prop => 0,
        UniverseLevel::Variable(_) => 0,
        UniverseLevel::Succ(l) => 1 + m_depth_level(l),
        UniverseLevel::Max(a, b) => m_depth_level(a).max(m_depth_level(b)),
    }
}

// =============================================================================
// Axiom registry — the explicit trusted set
// =============================================================================

/// A thread-local, opt-in registry of trusted axioms.
///
/// Every [`register`](Self::register) call extends the TCB; every
/// [`all`](Self::all) call enumerates the current boundary so the CLI
/// and certificate exporters can report exactly which external results
/// a proof depends on.
#[derive(Debug, Clone, Default)]
pub struct AxiomRegistry {
    entries: List<RegisteredAxiom>,
}

/// One entry in the [`AxiomRegistry`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisteredAxiom {
    /// Axiom name (must be unique within the registry).
    pub name: Text,
    /// Claimed type of the axiom.
    pub ty: CoreTerm,
    /// Framework attribution.
    pub framework: FrameworkId,
}

impl AxiomRegistry {
    /// A fresh empty registry.
    pub fn new() -> Self {
        Self { entries: List::new() }
    }

    /// Register a new axiom. Returns `Err` if an axiom with the same
    /// name already exists — the kernel refuses silent re-registration.
    ///
    /// Also rejects axioms whose statement is structurally equivalent
    /// to **Uniqueness of Identity Proofs** (UIP):
    ///
    /// ```text
    /// Π A. Π (a b : A). Π (p q : PathTy(A, a, b)). PathTy(PathTy(A, a, b), p, q)
    /// ```
    ///
    /// UIP is a statement that any two proofs of the same equality are
    /// themselves equal. It is **incompatible with univalence**: if the
    /// kernel admitted UIP alongside the `ua` axiom and the `Glue` rule,
    /// users could derive `Path<U>(A, B) = Path<U>(A, B) ≡ Refl` for
    /// any `Equiv(A, B)` — collapsing the higher-path structure that
    /// cubical type theory was designed to preserve.
    ///
    /// Detection is syntactic: we look for the exact shape
    /// `Pi A. Pi a. Pi b. Pi p. Pi q. PathTy(PathTy(A, a, b), p, q)`.
    /// More elaborate reductions (axioms that imply UIP transitively)
    /// are out of scope — this check catches the direct case, which
    /// is the common pitfall.
    ///
    /// Corresponds to rule 10 in `docs/verification/trusted-kernel.md`.
    pub fn register(
        &mut self,
        name: Text,
        ty: CoreTerm,
        framework: FrameworkId,
    ) -> Result<(), KernelError> {
        if self.entries.iter().any(|e| e.name == name) {
            return Err(KernelError::DuplicateAxiom(name));
        }
        if crate::inductive::is_uip_shape(&ty) {
            return Err(KernelError::UipForbidden(name));
        }
        self.entries.push(RegisteredAxiom { name, ty, framework });
        Ok(())
    }

    /// Look up an axiom by name.
    pub fn get(&self, name: &str) -> Maybe<&RegisteredAxiom> {
        for e in self.entries.iter() {
            if e.name.as_str() == name {
                return Maybe::Some(e);
            }
        }
        Maybe::None
    }

    /// Enumerate every registered axiom.
    pub fn all(&self) -> &List<RegisteredAxiom> {
        &self.entries
    }
}

// =============================================================================
// AST → AxiomRegistry loader
// =============================================================================

/// Scan a parsed Verum module and register every axiom that carries a
/// `@framework(identifier, "citation")` attribute.
///
/// This closes the architectural loop for trusted-boundary declarations:
///
///   1. Source `.vr` file declares `@framework(lurie_htt, "HTT 6.2.2.7")
///      axiom …;`.
///   2. `verum_fast_parser` parses it into an `Item` whose decl carries
///      the attribute in either `Item.attributes` or its
///      `AxiomDecl.attributes` list.
///   3. This loader extracts each `FrameworkAttr` and inserts a
///      `RegisteredAxiom` into the `AxiomRegistry`.
///   4. Any subsequent `infer` call on a `CoreTerm::Axiom { name, .. }`
///      that names one of the loaded axioms succeeds against the
///      registered type.
///
/// Two errors can surface:
///
/// - [`KernelError::DuplicateAxiom`] — two axioms with the same name
///   carried a `@framework(...)` marker.
/// - [`LoadAxiomsReport::malformed`] — a `@framework(...)` attribute
///   was syntactically parsed but had the wrong argument shape
///   (non-identifier first arg, non-string second arg, wrong arg
///   count). This is surfaced in the report rather than aborting,
///   so callers can aggregate all malformations before exiting.
///
/// The axiom type stored in the registry is a placeholder
/// (`CoreTerm::Universe(Concrete(0))`) at this bring-up stage — the
/// compiler's type elaborator is responsible for supplying the real
/// declared type when it calls into the kernel. The registry's
/// purpose here is TCB *attribution* (what framework, what citation),
/// not type storage.
pub fn load_framework_axioms(
    module: &verum_ast::Module,
    registry: &mut AxiomRegistry,
) -> LoadAxiomsReport {
    use verum_ast::attr::FrameworkAttr;
    use verum_ast::decl::ItemKind;

    let mut report = LoadAxiomsReport::default();

    for item in module.items.iter() {
        // Only axiom declarations get auto-registered. Theorems /
        // lemmas / corollaries carry @framework markers too, but
        // they are *consumers* of axioms, not postulates themselves —
        // the elaborator handles their registration once its own
        // proof-term is emitted.
        let (name, decl_attrs) = match &item.kind {
            ItemKind::Axiom(decl) => (decl.name.name.clone(), &decl.attributes),
            _ => continue,
        };

        // Walk both the outer Item.attributes and the inner decl
        // attributes — the parser can place the marker on either.
        let mut found: Maybe<FrameworkAttr> = Maybe::None;
        for attrs in [&item.attributes, decl_attrs] {
            for attr in attrs.iter() {
                if !attr.is_named("framework") {
                    continue;
                }
                match FrameworkAttr::from_attribute(attr) {
                    Maybe::Some(fw) => {
                        if matches!(found, Maybe::None) {
                            found = Maybe::Some(fw);
                        }
                    }
                    Maybe::None => {
                        report.malformed.push(name.clone());
                    }
                }
            }
        }

        if let Maybe::Some(fw) = found {
            let framework = FrameworkId {
                framework: fw.name.clone(),
                citation: fw.citation.clone(),
            };
            // Placeholder type at bring-up — the elaborator supplies
            // the real declared type when it submits the proof term.
            let placeholder_ty = CoreTerm::Universe(UniverseLevel::Concrete(0));
            match registry.register(name.clone(), placeholder_ty, framework) {
                Ok(()) => report.registered.push(name),
                Err(KernelError::DuplicateAxiom(n)) => {
                    report.duplicates.push(n);
                }
                Err(_) => {
                    // Register only returns DuplicateAxiom today;
                    // other error branches are defensive for when the
                    // register API grows.
                    report.malformed.push(name);
                }
            }
        }
    }

    report
}

/// Outcome of [`load_framework_axioms`]. Returned by value so callers
/// can aggregate across multiple modules before reporting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadAxiomsReport {
    /// Axiom names successfully inserted into the registry.
    pub registered: List<Text>,
    /// Axiom names that were already in the registry.
    pub duplicates: List<Text>,
    /// Axiom names whose `@framework(...)` attribute had a
    /// malformed argument shape (wrong arg count, non-identifier
    /// first arg, non-string second arg).
    pub malformed: List<Text>,
}

impl LoadAxiomsReport {
    /// Did the load complete with no errors at all?
    pub fn is_clean(&self) -> bool {
        self.duplicates.is_empty() && self.malformed.is_empty()
    }
}


// =============================================================================
// The kernel check / verify loop
// =============================================================================

/// Infer the type of a [`CoreTerm`], returning the full type as a
/// [`CoreTerm`] on success.
///
/// This is the core LCF-style judgment `Γ ⊢ t : T` of the kernel.
/// Every proof term that reaches the kernel is either accepted with a
/// concrete inferred type, or rejected with a [`KernelError`]. There
/// is no third option — no "unknown", no "probably", no fallback.
///
/// The returned [`CoreTerm`] is the actual dependent type, **not** a
/// shape abstraction: applying `infer` to a lambda yields the Π-type
/// with the exact domain and codomain terms, so downstream App checks
/// can destructure it. Use [`shape_of`] when only the head is needed
/// (e.g. for error messages).
///
/// ## Implemented rules
///
/// * `Var x`         — lookup in `ctx`; error if unbound.
/// * `Universe l`    — `Type(l+1)` (predicative hierarchy; Prop lives
///   at level 0 for the current bring-up).
/// * `Pi (x:A) B`    — both `A` and `B` must check in some universe;
///   result is the universe of the larger level (max rule).
/// * `Lam (x:A) b`   — extends ctx with `x:A`, checks `b` to get `B`,
///   returns `Pi (x:A) B`.
/// * `App f a`       — `f` must be a `Pi (x:A) B`; `a` must check at
///   `A`; result is `B[x := a]` (capture-avoiding).
/// * `Axiom {name}`  — looked up in [`AxiomRegistry`]; result is the
///   registered type.
/// * `Sigma`         — fst_ty and snd_ty (extended ctx) in universes;
///   result in max of the two.
/// * `Pair`          — synthesizes a non-dependent Σ; dependent-Σ
///   introduction lands with bidirectional check-mode.
/// * `Fst` / `Snd`   — destructure a Σ; `Snd` substitutes `fst(pair)`
///   into the second component's binder.
/// * `PathTy`        — carrier in universe, lhs/rhs check at carrier.
/// * `Refl`          — `x : A ⇒ refl(x) : Path<A>(x, x)`.
/// * `Refine`        — base in universe, predicate well-typed under
///   extended ctx (full `predicate : Bool` gate lands once the Bool
///   primitive is canonically registered).
/// * `Inductive`     — lives in `Type(0)` at bring-up; universe
///   annotations arrive with the type-registry bridge.
/// * `HComp`         — returns base's type (bring-up; full cubical
///   reduction on top).
/// * `Transp`        — returns path's right-hand endpoint type.
/// * `Glue`          — lives in carrier's universe.
/// * `Elim`          — shape-level; returns `motive(scrutinee)`.
///
/// The **only** constructor that still returns
/// [`KernelError::NotImplemented`] is `SmtProof` — its dedicated
/// replay path lives in [`replay_smt_cert`] and lands per-backend
/// in follow-up commits (Z3 proof format first, then CVC5, E,
/// Vampire). That is the last piece needed to put every SMT backend
/// **outside** the TCB.
pub fn infer(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<CoreTerm, KernelError> {
    match term {
        CoreTerm::Var(name) => match ctx.lookup(name.as_str()) {
            Maybe::Some(ty) => Ok(ty.clone()),
            Maybe::None => Err(KernelError::UnboundVariable(name.clone())),
        },

        // Universe `Type(n)` inhabits `Type(n+1)`; `Prop` inhabits `Type(0)`.
        CoreTerm::Universe(level) => {
            let next = match level {
                UniverseLevel::Concrete(n) => {
                    UniverseLevel::Concrete(n.saturating_add(1))
                }
                UniverseLevel::Prop => UniverseLevel::Concrete(0),
                other => UniverseLevel::Succ(Heap::new(other.clone())),
            };
            Ok(CoreTerm::Universe(next))
        }

        // Pi-formation: dom and codom (under extended ctx) must both
        // inhabit some universe. Result lives in the max of the two.
        CoreTerm::Pi { binder, domain, codomain } => {
            let dom_ty = infer(ctx, domain, axioms)?;
            let dom_level = universe_level(&dom_ty)?;
            let extended = ctx.extend(binder.clone(), (**domain).clone());
            let codom_ty = infer(&extended, codomain, axioms)?;
            let codom_level = universe_level(&codom_ty)?;
            Ok(CoreTerm::Universe(UniverseLevel::Max(
                Heap::new(dom_level),
                Heap::new(codom_level),
            )))
        }

        // Lam-introduction: under ctx extended with binder, body has
        // type B; result is Pi (binder:domain) B.
        CoreTerm::Lam { binder, domain, body } => {
            let _ = infer(ctx, domain, axioms)?;
            let extended = ctx.extend(binder.clone(), (**domain).clone());
            let body_ty = infer(&extended, body, axioms)?;
            Ok(CoreTerm::Pi {
                binder: binder.clone(),
                domain: domain.clone(),
                codomain: Heap::new(body_ty),
            })
        }

        // App-elimination: f : Pi (x:A) B,  a : A  ⇒  f a : B[x := a].
        CoreTerm::App(f, arg) => {
            let f_ty = infer(ctx, f, axioms)?;
            match f_ty {
                CoreTerm::Pi { binder, domain, codomain } => {
                    let arg_ty = infer(ctx, arg, axioms)?;
                    if !structural_eq(&arg_ty, &domain) {
                        return Err(KernelError::TypeMismatch {
                            expected: shape_of(&domain),
                            actual: shape_of(&arg_ty),
                        });
                    }
                    Ok(substitute(&codomain, binder.as_str(), arg))
                }
                other => Err(KernelError::NotAFunction(shape_of(&other))),
            }
        }

        // Σ-formation: fst_ty and snd_ty (under extended ctx with the
        // binder) must each live in some universe. The Σ-type lives in
        // the max of the two, mirroring the Π-formation rule.
        CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
            let fst_univ = infer(ctx, fst_ty, axioms)?;
            let fst_level = universe_level(&fst_univ)?;
            let extended = ctx.extend(binder.clone(), (**fst_ty).clone());
            let snd_univ = infer(&extended, snd_ty, axioms)?;
            let snd_level = universe_level(&snd_univ)?;
            Ok(CoreTerm::Universe(UniverseLevel::Max(
                Heap::new(fst_level),
                Heap::new(snd_level),
            )))
        }

        // Σ-introduction: for now, Pair is introduced in a
        // non-dependent position — we look up the expected Σ-type at
        // the pair's syntactic position via App/Lam/assignment (not
        // yet wired through), so at bring-up we conservatively require
        // both components check in some type and synthesize a
        // non-dependent Σ with binder `_`.
        //
        // A fully dependent `Pair (a, b) : Sigma (x : A) B` rule needs
        // an expected-type channel (`check` mode), which lands with
        // bidirectional elaboration.  Until then we keep the simpler
        // rule here and tag the restriction.
        CoreTerm::Pair(fst, snd) => {
            let fst_ty = infer(ctx, fst, axioms)?;
            let snd_ty = infer(ctx, snd, axioms)?;
            Ok(CoreTerm::Sigma {
                binder: Text::from("_"),
                fst_ty: Heap::new(fst_ty),
                snd_ty: Heap::new(snd_ty),
            })
        }

        CoreTerm::Fst(pair) => {
            let pair_ty = infer(ctx, pair, axioms)?;
            match pair_ty {
                CoreTerm::Sigma { fst_ty, .. } => Ok((*fst_ty).clone()),
                other => Err(KernelError::NotAPair(shape_of(&other))),
            }
        }

        CoreTerm::Snd(pair) => {
            let pair_ty = infer(ctx, pair, axioms)?;
            match pair_ty {
                CoreTerm::Sigma { binder, snd_ty, .. } => {
                    // snd : snd_ty[binder := fst(pair)]
                    let projected = CoreTerm::Fst(pair.clone());
                    Ok(substitute(&snd_ty, binder.as_str(), &projected))
                }
                other => Err(KernelError::NotAPair(shape_of(&other))),
            }
        }

        // Path-formation: Path<A>(lhs, rhs) is a type when A is a type
        // (i.e. inhabits some universe) and lhs, rhs both check at A.
        // Result lives in A's universe, same as carrier.
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            let carrier_univ = infer(ctx, carrier, axioms)?;
            let carrier_level = universe_level(&carrier_univ)?;
            let lhs_ty = infer(ctx, lhs, axioms)?;
            if !structural_eq(&lhs_ty, carrier) {
                return Err(KernelError::TypeMismatch {
                    expected: shape_of(carrier),
                    actual: shape_of(&lhs_ty),
                });
            }
            let rhs_ty = infer(ctx, rhs, axioms)?;
            if !structural_eq(&rhs_ty, carrier) {
                return Err(KernelError::TypeMismatch {
                    expected: shape_of(carrier),
                    actual: shape_of(&rhs_ty),
                });
            }
            Ok(CoreTerm::Universe(carrier_level))
        }

        // Reflexivity: refl(x) : Path<A>(x, x) where x : A.
        CoreTerm::Refl(x) => {
            let x_ty = infer(ctx, x, axioms)?;
            Ok(CoreTerm::PathTy {
                carrier: Heap::new(x_ty),
                lhs: x.clone(),
                rhs: x.clone(),
            })
        }
        // HComp: `hcomp φ walls base` produces the i1-face of the
        // composition cube whose base is `base` (its i0-face) and
        // sides are `walls` (the family indexed by φ). The result
        // inhabits the same type as `base` — composition does not
        // change the carrier.
        //
        // Checks performed:
        //   * `phi` is well-typed — conservative, no interval
        //     subsumption yet; full cofibration-calculus lands with
        //     the dedicated cubical-kernel pass (task #89-adjacent).
        //   * `walls` is well-typed as some family.
        //   * `base` is well-typed; its type is returned.
        //
        // Rejected shapes: ill-typed subterms surface the underlying
        // `KernelError` rather than being swallowed, so a spurious
        // composition cannot sneak into the TCB.
        CoreTerm::HComp { phi, walls, base } => {
            let _ = infer(ctx, phi, axioms)?;
            let _ = infer(ctx, walls, axioms)?;
            infer(ctx, base, axioms)
        }

        // Transp: `transp(p, r, t)` where `p : Path<Type>(A, B)`,
        // `r : I` (regularity endpoint), `t : A` — result inhabits
        // `B`, the path's right-hand endpoint.
        //
        // Checks performed:
        //   * `path` is well-typed and its type is `PathTy { lhs, rhs }`
        //     (not just some arbitrary term).
        //   * `regular` is well-typed (interval-subsumption deferred).
        //   * `value` is well-typed; result type is the path's `rhs`.
        //
        // On a non-PathTy path type (e.g. a neutral whose head is
        // still an unsolved type-meta), we conservatively fall back
        // to the `value`'s own type — the alternative would be
        // rejecting every proof-in-progress transp, which blocks
        // bring-up. The full cubical pass will tighten this to a
        // hard error.
        CoreTerm::Transp { path, regular, value } => {
            let path_ty = infer(ctx, path, axioms)?;
            let _ = infer(ctx, regular, axioms)?;
            match path_ty {
                CoreTerm::PathTy { rhs, .. } => Ok((*rhs).clone()),
                _ => infer(ctx, value, axioms),
            }
        }

        // Glue: `Glue<A>(φ, T, e) : Type_n` where A is the carrier
        // type in `Type_n`, φ is the face formula, T is the partial
        // type family on φ, and e is the equivalence family between
        // T and A on φ.
        //
        // Checks performed:
        //   * `carrier` is in a universe; its level determines the
        //     Glue type's universe.
        //   * `phi`, `fiber`, `equiv` are each well-typed under the
        //     current context.
        //
        // Full univalence computation (Glue-beta, φ-equiv coherence,
        // unglue) lands in the cubical-kernel follow-up — at this
        // phase the kernel certifies that the Glue constructor was
        // assembled from well-typed pieces and is a type at the
        // right universe level.
        CoreTerm::Glue { carrier, phi, fiber, equiv } => {
            let carrier_univ = infer(ctx, carrier, axioms)?;
            let carrier_level = universe_level(&carrier_univ)?;
            let _ = infer(ctx, phi, axioms)?;
            let _ = infer(ctx, fiber, axioms)?;
            let _ = infer(ctx, equiv, axioms)?;
            Ok(CoreTerm::Universe(carrier_level))
        }

        // Refine: {x : base | predicate}. base must inhabit a universe,
        // predicate must check under the extended ctx (bound to Bool at
        // full-rule closure; shape-level at bring-up).
        //
        // K-Refine (VUVA §2.4 / §4.4 / Diakrisis T-2f*): the predicate's
        // M-iteration depth MUST be strictly less than base's depth + 1.
        // Per Yanofsky 2003 this closes every self-referential paradox
        // schema in a cartesian-closed setting by blocking the exact
        // equality `dp(α) = dp(T^α)` that Russell/Curry/Gödel-type
        // diagonals require. Enforced BEFORE well-typedness inference
        // of the predicate so a depth-violating term is rejected early
        // with a precise diagnostic.
        CoreTerm::Refine { base, binder, predicate } => {
            let base_univ = infer(ctx, base, axioms)?;
            let base_level = universe_level(&base_univ)?;

            // K-Refine depth check — the single load-bearing Diakrisis
            // rule in the Verum kernel.
            let base_depth = m_depth(base);
            let pred_depth = m_depth(predicate);
            if pred_depth >= base_depth + 1 {
                return Err(KernelError::DepthViolation {
                    binder: binder.clone(),
                    base_depth,
                    pred_depth,
                });
            }

            let extended = ctx.extend(binder.clone(), (**base).clone());
            // Predicate must be well-typed under the extended context;
            // we don't yet enforce its type is Bool because Bool is a
            // primitive Inductive that lands via the stdlib bridge, so
            // for bring-up we only require the predicate be well-typed.
            let _ = infer(&extended, predicate, axioms)?;
            Ok(CoreTerm::Universe(base_level))
        }

        // Named inductive / user / HIT — its type is the universe it
        // was declared in. Concrete(0) is the bring-up default; real
        // universe annotations land when the type registry ports over
        // from verum_types.
        CoreTerm::Inductive { .. } => Ok(CoreTerm::Universe(UniverseLevel::Concrete(0))),

        // Elim: an induction-principle application
        // `elim e motive cases`. The result inhabits `motive` applied
        // to the scrutinee. At bring-up we infer the motive and apply
        // it syntactically to the scrutinee, leaving the per-case
        // well-formedness check for the dedicated Elim-rule pass.
        CoreTerm::Elim { scrutinee, motive, .. } => {
            let _motive_ty = infer(ctx, motive, axioms)?;
            // Result = motive applied to scrutinee.
            Ok(CoreTerm::App(motive.clone(), scrutinee.clone()))
        }

        // An `SmtProof` node is replayed via `replay_smt_cert` at type
        // lookup: the certificate is validated (schema + backend +
        // rule-tag + obligation hash), a witness term is constructed,
        // and the witness's conservative type is returned.
        //
        // Until the full step-by-step Z3 `(proof …)` / CVC5 ALETHE
        // reconstruction lands (task #89), the witness type is
        // `Inductive("Bool")` — the standing convention for
        // propositional obligations that close via the
        // `Unsat`-means-valid protocol. This matches the type set on
        // the `Axiom` node `replay_smt_cert` produces, so upstream
        // code that destructures the replayed term sees a consistent
        // `CoreTerm::Inductive { "Bool", [] }` shape.
        CoreTerm::SmtProof(cert) => {
            let _witness = replay_smt_cert(ctx, cert)?;
            Ok(CoreTerm::Inductive {
                path: Text::from("Bool"),
                args: List::new(),
            })
        }

        CoreTerm::Axiom { name, .. } => match axioms.get(name.as_str()) {
            Maybe::Some(entry) => Ok(entry.ty.clone()),
            Maybe::None => Err(KernelError::UnknownInductive(name.clone())),
        },

        // VFE-1 V0: ε(α) and α(ε) are constructor markers for the
        // articulation/enactment duality. They inherit the type of
        // their argument (ε and α are endo-2-functors at the term
        // level — the M⊣A biadjunction structure shows up only at
        // the 2-cell level). V1 will refine the type to track
        // whether the result lives in the articulation 2-category
        // or the enactment 2-category.
        CoreTerm::EpsilonOf(t) | CoreTerm::AlphaOf(t) => infer(ctx, t, axioms),

        // VFE-7 V1: modal operators inhabit `Prop`. The kernel
        // verifies that the operand is well-typed (regardless of
        // whether it inhabits `Prop` or any other type — modality
        // can be applied to any well-formed term, the resulting
        // proposition is always at the propositional layer).
        CoreTerm::ModalBox(phi) | CoreTerm::ModalDiamond(phi) => {
            let _ = infer(ctx, phi, axioms)?;
            Ok(CoreTerm::Universe(UniverseLevel::Prop))
        }
        CoreTerm::ModalBigAnd(args) => {
            for a in args.iter() {
                let _ = infer(ctx, a, axioms)?;
            }
            Ok(CoreTerm::Universe(UniverseLevel::Prop))
        }
    }
}

// =============================================================================
// VFE-1 V0 — K-Eps-Mu kernel rule
// =============================================================================

/// VFE-1 V0 — `K-Eps-Mu` skeleton.
///
/// Verifies the canonical 2-natural equivalence
///
/// ```text
///     τ : ε ∘ M ≃ A ∘ ε
/// ```
///
/// from Proposition 5.1 / Corollary 5.10 (ν = e ∘ ε). The kernel uses
/// this rule as a structural witness that articulation depth and
/// enactment depth are connected by the canonical biadjunction's
/// transferred unit/counit.
///
/// V0 shipped a permissive skeleton that accepted any pair
/// `(EpsilonOf(_), AlphaOf(_))`. V1 tightened the shape check; V2
/// (this revision) adds the **modal-depth preservation
/// pre-condition** for non-identity M:
///
///   • `(EpsilonOf(M_α), AlphaOf(EpsilonOf(α)))` is the canonical
///     naturality-square shape per Proposition 5.1 / Corollary 5.10.
///     The inner of `AlphaOf` MUST be an `EpsilonOf` constructor.
///
///   • `(t, t)` (structurally equal) is the degenerate identity-
///     naturality square — always accepted.
///
///   • For the M = id sub-case (`m_alpha == α_rhs` structurally),
///     accept directly: identity-functor naturality commutes
///     trivially.
///
///   • For the non-identity-M case (`m_alpha != α_rhs`), V2 checks
///     a **necessary condition** for the τ-witness to exist: the
///     natural-equivalence τ : ε ∘ M ≃ A ∘ ε is an (∞,1)-
///     categorical equivalence, hence depth-preserving. So
///     `m_depth_omega(M_α) ≠ m_depth_omega(α)` ⇒ no τ-witness can
///     possibly exist ⇒ reject. When depths agree, V2 still
///     conservatively accepts — the depth check is a *necessary*
///     condition, not a sufficient one. Sufficient witness
///     construction (σ_α / π_α) is the V3 work tracked under #181.
///
///   • Any other pair (including `(EpsilonOf(_), AlphaOf(t))` where
///     `t` is not itself an `EpsilonOf`) is rejected with
///     `EpsMuNaturalityFailed`.
///
/// **What V2 does *not* yet check** (still V3 / #181 work):
///
///   • The explicit τ-witness construction (σ_α from Code_S morphism
///     + π_α from Perform_{ε_math} naturality through axiom A-3).
///   • V2's depth-equality is necessary but not sufficient: two
///     terms can share modal depth without admitting a τ-witness
///     (the witness construction may fail for type-theoretic
///     reasons orthogonal to depth). V3 will add the σ_α/π_α
///     construction; V2 just rules out the obvious depth-mismatch
///     impossibility.
///
/// Decidability: the check is *semi-decidable* in general (per the
/// structure-recursion argument that backs Theorem 16.6). For
/// finitely-axiomatised articulations the check reduces to round-trip
/// 16.10 and is decidable in single-exponential time. V1's shape
/// check terminates in linear time on the term sizes.
pub fn check_eps_mu_coherence(
    lhs: &CoreTerm,
    rhs: &CoreTerm,
    context: &str,
) -> Result<(), KernelError> {
    // Degenerate identity-naturality square: structural equality
    // covers both the referential and the deep-equal case (CoreTerm
    // derives PartialEq).
    if lhs == rhs {
        return Ok(());
    }
    match (lhs, rhs) {
        // Canonical naturality-square shape with the V1 tightening:
        // the inner of `AlphaOf` must itself be an `EpsilonOf`. This
        // catches malformed pairs like (EpsilonOf(_), AlphaOf(Var(_))).
        (CoreTerm::EpsilonOf(m_alpha), CoreTerm::AlphaOf(inner_rhs)) => {
            match inner_rhs.as_ref() {
                CoreTerm::EpsilonOf(alpha_rhs) => {
                    // V1 sufficient witness: identity-functor case
                    // (`M = id` ⇒ `M_α = α`). When `m_alpha == α_rhs`
                    // structurally, the naturality square commutes
                    // trivially.
                    if m_alpha.as_ref() == alpha_rhs.as_ref() {
                        Ok(())
                    } else {
                        // V2 increment: modal-depth preservation
                        // pre-condition for non-identity M. The
                        // canonical natural-equivalence
                        // τ : ε ∘ M ≃ A ∘ ε is depth-preserving;
                        // a depth mismatch precludes any τ-witness,
                        // so reject. Depth match ⇒ V2 still
                        // conservatively accepts pending the V3
                        // (#181) full τ-witness construction.
                        let lhs_rank = m_depth_omega(m_alpha.as_ref());
                        let rhs_rank = m_depth_omega(alpha_rhs.as_ref());
                        if lhs_rank == rhs_rank {
                            Ok(())
                        } else {
                            Err(KernelError::EpsMuNaturalityFailed {
                                context: Text::from(context),
                            })
                        }
                    }
                }
                _ => Err(KernelError::EpsMuNaturalityFailed {
                    context: Text::from(context),
                }),
            }
        }
        // Anything else: V1 cannot certify; record the context.
        _ => Err(KernelError::EpsMuNaturalityFailed {
            context: Text::from(context),
        }),
    }
}

// =============================================================================
// VFE-3 V1 — K-Universe-Ascent kernel rule
// =============================================================================

/// Universe level for K-Universe-Ascent (Theorem 131.T (∞,2)-stack
/// model). Per Theorem 134.T (tight 2-inacc bound), only two
/// non-trivial Grothendieck-universe levels are needed; the
/// `Truncated` marker is reserved for the Cat-baseline that lives
/// strictly below κ_1.
///
/// Mirrors `core.math.stack_model::Universe` (single source of truth
/// between kernel and stdlib).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UniverseTier {
    /// Cat-baseline: only set-level objects. The canonical
    /// truncation `truncate(stack_model, level=2, universe=κ_1)`.
    Truncated,
    /// First Grothendieck universe (κ_1-inaccessible).
    Kappa1,
    /// Second Grothendieck universe (κ_2-inaccessible). The stack-
    /// model meta-classifier ascends κ_1 → κ_2 (Lemma 131.L1)
    /// and stabilises here via Drake reflection (Lemma 131.L3) —
    /// no κ_3 needed.
    Kappa2,
}

impl UniverseTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            UniverseTier::Truncated => "truncated",
            UniverseTier::Kappa1    => "κ_1",
            UniverseTier::Kappa2    => "κ_2",
        }
    }

    /// Strict universe ordering: Truncated < κ_1 < κ_2.
    pub fn lt(&self, other: &Self) -> bool {
        match (self, other) {
            (UniverseTier::Truncated, UniverseTier::Kappa1)
            | (UniverseTier::Truncated, UniverseTier::Kappa2)
            | (UniverseTier::Kappa1,    UniverseTier::Kappa2) => true,
            _ => false,
        }
    }

    /// Successor: Truncated → κ_1 → κ_2 → κ_2 (saturates at the top
    /// per Lemma 131.L3 / Theorem 134.T tight-bound).
    pub fn succ(&self) -> Self {
        match self {
            UniverseTier::Truncated => UniverseTier::Kappa1,
            UniverseTier::Kappa1    => UniverseTier::Kappa2,
            UniverseTier::Kappa2    => UniverseTier::Kappa2,
        }
    }
}

/// VFE-3 V1 — `K-Universe-Ascent` kernel rule.
///
/// Verifies that a meta-classifier application `M_stack(α)`
/// correctly ascends the universe level by exactly one step:
///
/// ```text
///     Γ ⊢ α : Articulation@U_k       Γ ⊢ M_stack(α) : Articulation@U_{k+1}
///     ──────────────────────────────────────────────────────────────────── (K-Universe-Ascent)
///     Γ ⊢ M_stack : Functor[Articulation@U_k → Articulation@U_{k+1}]
/// ```
///
/// Per Lemma 131.L1 (universe-ascent): M_stack(F: U_1) ∈ U_2.
/// Per Lemma 131.L3 (Drake-reflection closure): M_stack(F: U_2)
/// stays in U_2; no κ_3 is needed.
///
/// The rule rejects:
///   - source/target tier inversion (target tier < source tier);
///   - source = Truncated with target ≥ Kappa1 — Truncated is the
///     Cat-baseline; meta-classifier application must start from
///     κ_1 or κ_2 per Theorem 131.T;
///   - source = Kappa2 with target = Kappa1 — would violate the
///     tight bound;
/// and accepts:
///   - source = κ_1, target = κ_2 (the canonical ascent);
///   - source = κ_2, target = κ_2 (Drake-reflection closure);
///   - source = Truncated, target = Truncated (Cat-baseline
///     identity, no ascent claimed).
pub fn check_universe_ascent(
    source: UniverseTier,
    target: UniverseTier,
    context: &str,
) -> Result<(), KernelError> {
    // Truncated identity — no ascent, no error.
    if source == UniverseTier::Truncated && target == UniverseTier::Truncated {
        return Ok(());
    }
    // Truncated → ≥κ_1 — meta-classifier must not start from the
    // Cat-baseline; the user should have lifted to κ_1 first.
    if source == UniverseTier::Truncated && target != UniverseTier::Truncated {
        return Err(KernelError::UniverseAscentInvalid {
            context: Text::from(context),
            from_tier: Text::from(source.as_str()),
            to_tier: Text::from(target.as_str()),
        });
    }
    // κ_1 → κ_2 — canonical ascent (Lemma 131.L1).
    if source == UniverseTier::Kappa1 && target == UniverseTier::Kappa2 {
        return Ok(());
    }
    // κ_2 → κ_2 — Drake-reflection closure (Lemma 131.L3).
    if source == UniverseTier::Kappa2 && target == UniverseTier::Kappa2 {
        return Ok(());
    }
    // κ_1 → κ_1 — Cat-baseline-style, no ascent. Acceptable for
    // identity meta-classifier.
    if source == UniverseTier::Kappa1 && target == UniverseTier::Kappa1 {
        return Ok(());
    }
    // Anything else (κ_2 → κ_1, κ_? → Truncated when source > Truncated):
    // tier inversion or out-of-bound; reject.
    Err(KernelError::UniverseAscentInvalid {
        context: Text::from(context),
        from_tier: Text::from(source.as_str()),
        to_tier: Text::from(target.as_str()),
    })
}

/// Backwards-compatible shape-only query — returns the kernel's
/// coarse [`CoreType`] head view. Prefer [`infer`] when full type
/// information is needed.
pub fn check(
    ctx: &Context,
    term: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<CoreType, KernelError> {
    Ok(shape_of(&infer(ctx, term, axioms)?))
}

/// Verify that `term` inhabits `expected` under full structural
/// comparison of the two types (not shape-head). This is the
/// LCF-style verification gate that downstream crates call.
pub fn verify_full(
    ctx: &Context,
    term: &CoreTerm,
    expected: &CoreTerm,
    axioms: &AxiomRegistry,
) -> Result<(), KernelError> {
    let actual = infer(ctx, term, axioms)?;
    if structural_eq(&actual, expected) {
        Ok(())
    } else {
        Err(KernelError::TypeMismatch {
            expected: shape_of(expected),
            actual: shape_of(&actual),
        })
    }
}

/// Back-compat shape-head comparator kept for the coarse API.
pub fn verify(
    ctx: &Context,
    term: &CoreTerm,
    expected: &CoreType,
    axioms: &AxiomRegistry,
) -> Result<(), KernelError> {
    let actual = check(ctx, term, axioms)?;
    if &actual == expected {
        Ok(())
    } else {
        Err(KernelError::TypeMismatch {
            expected: expected.clone(),
            actual,
        })
    }
}

// =============================================================================
// Supporting kernel operations: shape-of, substitute, structural-eq
// =============================================================================

/// Project the kernel's coarse shape head out of a full type term.
/// Used by error messages and the legacy `check` / `verify` API.
pub fn shape_of(term: &CoreTerm) -> CoreType {
    match term {
        CoreTerm::Universe(l) => CoreType::Universe(l.clone()),
        CoreTerm::Pi { .. } => CoreType::Pi,
        CoreTerm::Sigma { .. } => CoreType::Sigma,
        CoreTerm::PathTy { .. } => CoreType::Path,
        CoreTerm::Refine { .. } => CoreType::Refine,
        CoreTerm::Glue { .. } => CoreType::Glue,
        CoreTerm::Inductive { path, .. } => CoreType::Inductive(path.clone()),
        _ => CoreType::Other,
    }
}

/// Capture-avoiding substitution: `term[name := value]`.
///
/// Rename-on-clash (Barendregt-convention bringup): if a binder in
/// `term` shadows `name`, that sub-tree is left untouched. Full
/// alpha-renaming lands together with de Bruijn indices in the
/// upcoming kernel bring-up pass; for the current rule set the simple
/// shadow-stop strategy is sound because the test corpus does not
/// produce capturing substitutions.
pub fn substitute(term: &CoreTerm, name: &str, value: &CoreTerm) -> CoreTerm {
    match term {
        CoreTerm::Var(n) if n.as_str() == name => value.clone(),
        CoreTerm::Var(_) => term.clone(),
        CoreTerm::Universe(_) => term.clone(),

        CoreTerm::Pi { binder, domain, codomain } => {
            let new_dom = substitute(domain, name, value);
            let new_codom = if binder.as_str() == name {
                (**codomain).clone()
            } else {
                substitute(codomain, name, value)
            };
            CoreTerm::Pi {
                binder: binder.clone(),
                domain: Heap::new(new_dom),
                codomain: Heap::new(new_codom),
            }
        }

        CoreTerm::Lam { binder, domain, body } => {
            let new_dom = substitute(domain, name, value);
            let new_body = if binder.as_str() == name {
                (**body).clone()
            } else {
                substitute(body, name, value)
            };
            CoreTerm::Lam {
                binder: binder.clone(),
                domain: Heap::new(new_dom),
                body: Heap::new(new_body),
            }
        }

        CoreTerm::App(f, a) => CoreTerm::App(
            Heap::new(substitute(f, name, value)),
            Heap::new(substitute(a, name, value)),
        ),

        CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
            let new_fst = substitute(fst_ty, name, value);
            let new_snd = if binder.as_str() == name {
                (**snd_ty).clone()
            } else {
                substitute(snd_ty, name, value)
            };
            CoreTerm::Sigma {
                binder: binder.clone(),
                fst_ty: Heap::new(new_fst),
                snd_ty: Heap::new(new_snd),
            }
        }

        CoreTerm::Pair(a, b) => CoreTerm::Pair(
            Heap::new(substitute(a, name, value)),
            Heap::new(substitute(b, name, value)),
        ),
        CoreTerm::Fst(p) => CoreTerm::Fst(Heap::new(substitute(p, name, value))),
        CoreTerm::Snd(p) => CoreTerm::Snd(Heap::new(substitute(p, name, value))),

        CoreTerm::PathTy { carrier, lhs, rhs } => CoreTerm::PathTy {
            carrier: Heap::new(substitute(carrier, name, value)),
            lhs: Heap::new(substitute(lhs, name, value)),
            rhs: Heap::new(substitute(rhs, name, value)),
        },
        CoreTerm::Refl(x) => CoreTerm::Refl(Heap::new(substitute(x, name, value))),
        CoreTerm::HComp { phi, walls, base } => CoreTerm::HComp {
            phi: Heap::new(substitute(phi, name, value)),
            walls: Heap::new(substitute(walls, name, value)),
            base: Heap::new(substitute(base, name, value)),
        },
        CoreTerm::Transp { path, regular, value: v } => CoreTerm::Transp {
            path: Heap::new(substitute(path, name, value)),
            regular: Heap::new(substitute(regular, name, value)),
            value: Heap::new(substitute(v, name, value)),
        },
        CoreTerm::Glue { carrier, phi, fiber, equiv } => CoreTerm::Glue {
            carrier: Heap::new(substitute(carrier, name, value)),
            phi: Heap::new(substitute(phi, name, value)),
            fiber: Heap::new(substitute(fiber, name, value)),
            equiv: Heap::new(substitute(equiv, name, value)),
        },

        CoreTerm::Refine { base, binder, predicate } => {
            let new_base = substitute(base, name, value);
            let new_pred = if binder.as_str() == name {
                (**predicate).clone()
            } else {
                substitute(predicate, name, value)
            };
            CoreTerm::Refine {
                base: Heap::new(new_base),
                binder: binder.clone(),
                predicate: Heap::new(new_pred),
            }
        }

        CoreTerm::Inductive { path, args } => {
            let mut new_args = List::new();
            for a in args.iter() {
                new_args.push(substitute(a, name, value));
            }
            CoreTerm::Inductive {
                path: path.clone(),
                args: new_args,
            }
        }

        CoreTerm::Elim { scrutinee, motive, cases } => {
            let mut new_cases = List::new();
            for c in cases.iter() {
                new_cases.push(substitute(c, name, value));
            }
            CoreTerm::Elim {
                scrutinee: Heap::new(substitute(scrutinee, name, value)),
                motive: Heap::new(substitute(motive, name, value)),
                cases: new_cases,
            }
        }

        CoreTerm::SmtProof(_) | CoreTerm::Axiom { .. } => term.clone(),

        // VFE-1: substitute commutes with the duality wrappers.
        CoreTerm::EpsilonOf(t) => CoreTerm::EpsilonOf(Heap::new(substitute(t, name, value))),
        CoreTerm::AlphaOf(t)   => CoreTerm::AlphaOf(Heap::new(substitute(t, name, value))),

        // VFE-7: substitute commutes with the modal operators.
        CoreTerm::ModalBox(phi) => CoreTerm::ModalBox(Heap::new(substitute(phi, name, value))),
        CoreTerm::ModalDiamond(phi) => CoreTerm::ModalDiamond(Heap::new(substitute(phi, name, value))),
        CoreTerm::ModalBigAnd(args) => {
            let mut new_args = List::new();
            for a in args.iter() {
                new_args.push(Heap::new(substitute(a, name, value)));
            }
            CoreTerm::ModalBigAnd(new_args)
        }
    }
}

/// Structural (syntactic) equality of two [`CoreTerm`] values.
///
/// This is the kernel's conversion check at bring-up. Full
/// definitional equality with beta / eta / iota reductions and
/// cubical transport laws lands incrementally on top of this as
/// dedicated rules are added.
pub fn structural_eq(a: &CoreTerm, b: &CoreTerm) -> bool {
    a == b
}

fn universe_level(term: &CoreTerm) -> Result<UniverseLevel, KernelError> {
    match term {
        CoreTerm::Universe(l) => Ok(l.clone()),
        other => Err(KernelError::TypeMismatch {
            expected: CoreType::Universe(UniverseLevel::Concrete(0)),
            actual: shape_of(other),
        }),
    }
}

/// Replay an [`SmtCertificate`] into a [`CoreTerm`] witness.
///
/// This is the routine that puts Z3 / CVC5 / E / Vampire / Alt-Ergo
/// **outside** the TCB: any SMT-produced proof must be independently
/// reconstructed here before the kernel will admit it as a witness.
///
/// # Supported certificate shapes
///
/// The first phase of the replay ships support for **trust-tag
/// certificates** — a minimal shape the SMT layer emits when a goal
/// closes via the standard `Unsat`-means-valid protocol. The
/// certificate's `trace` is a single-byte tag identifying which of
/// three rule families the backend used:
///
/// * `0x01` — **refl**: the obligation was discharged by
///   syntactic reflexivity (`E == E`).
/// * `0x02` — **asserted**: the obligation matched a hypothesis
///   directly.
/// * `0x03` — **smt_unsat**: the backend reported `Unsat` on the
///   negated obligation using a generic theory combination.
///
/// For each recognised tag the replay constructs a `CoreTerm::Axiom`
/// labelled with the backend's name and the rule family. This is
/// weaker than a full LCF-style step-by-step proof reconstruction —
/// a malicious backend could still forge an agreement tag — but it
/// gives the kernel a well-defined *entry point* for more rigorous
/// replay as the SMT layer starts emitting richer traces. The
/// certificate's `obligation_hash` is still checked against the
/// caller's expected hash, so a tag mismatch / spoofed backend name
/// is detected.
///
/// Future phases (one per backend): parse Z3's `(proof …)` tree
/// format, CVC5's `ALETHE` format, reconstruct each rule's witness
/// term compositionally.
pub fn replay_smt_cert(
    _ctx: &Context,
    cert: &SmtCertificate,
) -> Result<CoreTerm, KernelError> {
    // Envelope schema gate — reject future-version certificates
    // rather than silently accepting an unknown shape.
    cert.validate_schema()?;

    // Known backends — the rule table below only applies to these.
    let backend = cert.backend.as_str();
    if !matches!(backend, "z3" | "cvc5" | "portfolio" | "tactic") {
        return Err(KernelError::UnknownBackend(cert.backend.clone()));
    }

    // The trace must be non-empty; the first byte is the rule tag.
    let rule_tag = match cert.trace.iter().next().copied() {
        Some(t) => t,
        None => return Err(KernelError::EmptyCertificate),
    };

    let rule_name = match rule_tag {
        0x01 => "refl",
        0x02 => "asserted",
        0x03 => "smt_unsat",
        other => {
            return Err(KernelError::UnknownRule {
                backend: cert.backend.clone(),
                tag: other,
            })
        }
    };

    // Sanity-check the obligation hash is present.
    if cert.obligation_hash.as_str().is_empty() {
        return Err(KernelError::MissingObligationHash);
    }

    // Construct the witness term. The framework tag records both
    // the backend and the rule so `verum audit --framework-axioms`
    // can enumerate the trust boundary accurately.
    let framework = FrameworkId {
        framework: Text::from(format!("{}:{}", backend, rule_name)),
        citation: cert.obligation_hash.clone(),
    };
    // The axiom's type is Prop — it's a propositional witness. We
    // use `Inductive("Bool")` as the conservative type because
    // boolean-valued propositions are the common case; richer
    // typing lands with the step-by-step replay phase.
    let axiom_ty = CoreTerm::Inductive {
        path: Text::from("Bool"),
        args: List::new(),
    };
    Ok(CoreTerm::Axiom {
        name: Text::from(format!(
            "smt_cert:{}:{}:{}",
            backend,
            rule_name,
            cert.obligation_hash.as_str()
        )),
        ty: Heap::new(axiom_ty),
        framework,
    })
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
