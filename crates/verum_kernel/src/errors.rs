//! Kernel error type — the diagnostic surface for ill-typed proof terms.
//!
//! Kernel errors are **never** rescued by downstream passes. If you
//! see one, either the proof is wrong or a non-trusted component
//! (tactic, elaborator, SMT backend) produced a malformed term.
//!
//! Split out of `lib.rs` for auditability (#198): grouping all error
//! variants in a single file makes the diagnostic surface trivially
//! greppable for documentation, format-string consistency checks,
//! and red-team review of the trusted-base error catalogue.

use thiserror::Error;
use verum_common::Text;

use crate::CoreType;

/// The error type reported by the kernel on ill-typed proof terms.
///
/// Kernel errors are **never** rescued by downstream passes — if you
/// see one, either the proof is wrong or a non-trusted component
/// (tactic, elaborator, SMT backend) produced a malformed term.
#[derive(Debug, Clone, Error, PartialEq)]
pub enum KernelError {
    /// Variable used without a binding in scope.
    #[error("unbound variable: {0}")]
    UnboundVariable(Text),

    /// Application where the function part is not a Π-type.
    #[error("application expected a Pi type, got {0:?}")]
    NotAFunction(CoreType),

    /// Projection where the argument is not a Σ-pair.
    #[error("projection expected a Sigma type, got {0:?}")]
    NotAPair(CoreType),

    /// Path eliminator applied to a non-path term.
    #[error("path eliminator expected a Path type, got {0:?}")]
    NotAPath(CoreType),

    /// Type-mismatch between checked term and expected type.
    #[error("type mismatch: expected {expected:?}, got {actual:?}")]
    TypeMismatch {
        /// The type that was expected from context.
        expected: CoreType,
        /// The type that was actually produced.
        actual: CoreType,
    },

    /// Reference to an inductive type that has not been declared.
    #[error("unknown inductive type: {0}")]
    UnknownInductive(Text),

    /// Attempted re-registration of an axiom that already exists.
    #[error("duplicate axiom registration: {0}")]
    DuplicateAxiom(Text),

    /// An inductive declaration violates strict positivity (VVA §7.3
    /// `K-Pos` rule). The recursive occurrence of the type's own name
    /// inside a constructor's argument appears in a *negative*
    /// position (left of an arrow) — admitting such a definition is
    /// inconsistent (Berardi 1998). The kernel rejects the
    /// declaration outright; it does not partially admit a non-strict
    /// variant.
    ///
    /// `position` is a human-readable description of where the
    /// violation occurs — e.g. `"left of arrow inside constructor 'Wrap'
    /// arg #1"` — for diagnostic copy.
    #[error("strict positivity violation in inductive '{type_name}': constructor '{constructor}' has '{type_name}' in {position}")]
    PositivityViolation {
        /// Name of the inductive being declared.
        type_name: Text,
        /// Name of the offending constructor.
        constructor: Text,
        /// Human-readable description of the violation site.
        position: Text,
    },

    /// An inductive type's name was registered twice. Like
    /// [`Self::DuplicateAxiom`], the kernel refuses silent
    /// re-registration.
    #[error("duplicate inductive registration: {0}")]
    DuplicateInductive(Text),

    /// An SMT certificate failed to replay as a valid proof term.
    #[error("SMT certificate replay failed: {reason}")]
    SmtReplayFailed {
        /// Human-readable replay-failure reason.
        reason: Text,
    },

    /// A [`crate::CoreTerm`] constructor's checker has not been
    /// implemented yet. During the kernel bring-up period this is
    /// the expected failure mode for constructors still being ported.
    #[error("kernel check not yet implemented for {0}")]
    NotImplemented(&'static str),

    /// An SMT certificate referenced a backend the kernel doesn't
    /// recognise. Certificate replay requires the backend tag to
    /// match one of the registered solver identifiers.
    #[error("kernel: unknown SMT backend '{0}'")]
    UnknownBackend(Text),

    /// An SMT certificate's trace was empty — no rule tag to
    /// dispatch on.
    #[error("kernel: SMT certificate trace is empty")]
    EmptyCertificate,

    /// The first byte of the certificate trace is not a known
    /// rule tag for the certificate's backend.
    #[error("kernel: unknown rule tag {tag:#x} for backend '{backend}'")]
    UnknownRule {
        /// The backend that produced the certificate.
        backend: Text,
        /// The unrecognised rule-tag byte.
        tag: u8,
    },

    /// A certificate arrived without an obligation hash, so the
    /// kernel cannot cross-check that the certificate matches the
    /// goal the caller intended to prove.
    #[error("kernel: SMT certificate missing obligation hash")]
    MissingObligationHash,

    /// A certificate's envelope schema version is newer than the
    /// kernel understands. See [`crate::CERTIFICATE_SCHEMA_VERSION`]
    /// for what the kernel can replay; schema-version bumps go
    /// through task #90's cross-tool CI matrix before they ship.
    #[error(
        "kernel: unsupported certificate schema version {found} \
         (max supported: {max_supported})"
    )]
    UnsupportedCertificateSchema {
        /// Schema version found in the certificate.
        found: u32,
        /// Highest schema version this kernel build supports.
        max_supported: u32,
    },

    /// An axiom whose statement reduces to Uniqueness of Identity
    /// Proofs — `∀A, ∀(a b: A), ∀(p q: a = b), p = q`. UIP is
    /// incompatible with univalence and is explicitly rejected to
    /// preserve HoTT soundness (rule 10 in the trusted-kernel
    /// spec). Use `PathTy` and cubical rules for nontrivial
    /// equality proofs instead.
    #[error("kernel: axiom '{0}' is equivalent to UIP and is rejected (rule 10); use Path types instead")]
    UipForbidden(Text),

    /// A refinement type `{x : base | P(x)}` violates Diakrisis T-2f*
    /// depth-stratification: `dp(P) >= dp(base) + 1`. This is the
    /// Yanofsky paradox-immunity rule imported by VVA §2.4 as
    /// `K-Refine` — comprehension is admissible only when the
    /// predicate's M-iteration depth is strictly less than the
    /// comprehended object's depth.
    ///
    /// See:
    ///   - `internal/specs/verification-architecture.md` §2.4, §4.4
    ///   - Diakrisis `docs/02-canonical-primitive/02-axiomatics.md` T-2f*
    ///   - Yanofsky N.S. 2003. *A Universal Approach to Self-Referential
    ///     Paradoxes, Incompleteness and Fixed Points.*
    #[error(
        "kernel: K-Refine depth violation: predicate depth {pred_depth} \
         must be strictly less than base depth {base_depth} + 1 \
         (Diakrisis T-2f* / Yanofsky paradox-immunity)"
    )]
    DepthViolation {
        /// Bound variable name in the refinement.
        binder: Text,
        /// Computed `dp(base)`.
        base_depth: usize,
        /// Computed `dp(predicate)`.
        pred_depth: usize,
    },

    /// VVA-1 V0 — `K-Eps-Mu` naturality witness construction failed.
    /// The kernel attempted to verify the canonical 2-natural
    /// equivalence τ : ε ∘ M ≃ A ∘ ε of Proposition 5.1 / Corollary
    /// 5.10 and could not produce the τ-witness for the supplied
    /// articulation. V0 ships the constructor + skeleton check; V1
    /// will wire the full naturality proof and reduce this error to
    /// concrete diagnostic content.
    #[error("kernel: K-Eps-Mu naturality witness failed: {context}")]
    EpsMuNaturalityFailed {
        /// Human-readable context describing where the τ-witness
        /// construction broke (e.g., articulation name).
        context: Text,
    },

    /// VVA-7 V0 — `K-Refine-omega` modal-depth bound exceeded. A
    /// refinement type's predicate has ordinal modal-depth `md^ω`
    /// strictly greater than the base type's depth + 1, violating
    /// the transfinite stratification of Theorem 136.T (T-2f***).
    /// V0 ships the constructor; V1 wires the full ordinal-depth
    /// computation with well-founded recursion per Lemma 136.L0.
    #[error(
        "kernel: K-Refine-omega modal-depth violation: predicate \
         md^ω-rank '{pred_rank}' exceeds base md^ω-rank '{base_rank}' + 1 \
         (Theorem 136.T transfinite stratification)"
    )]
    ModalDepthExceeded {
        /// Bound variable name in the refinement.
        binder: Text,
        /// Computed `md^ω(base)` rendered as ordinal text.
        base_rank: Text,
        /// Computed `md^ω(predicate)` rendered as ordinal text.
        pred_rank: Text,
    },

    /// VVA-3 V1 — `K-Universe-Ascent` rule rejected an invalid
    /// universe transition. Meta-classifier application
    /// `M_stack(α)` must ascend universe levels in the canonical
    /// κ-tower per Theorem 131.T: Truncated → Truncated (Cat-id),
    /// κ_1 → κ_1 (id), κ_1 → κ_2 (Lemma 131.L1 ascent), or
    /// κ_2 → κ_2 (Lemma 131.L3 Drake-reflection closure). Any
    /// other transition (tier inversion, Truncated → κ_*, κ_2 →
    /// κ_1) is rejected here. Fields are renamed away from
    /// `source`/`target` to avoid `thiserror`'s implicit
    /// error-chain convention on those names.
    #[error(
        "kernel: K-Universe-Ascent invalid transition: '{from_tier}' → '{to_tier}' \
         is not a valid κ-tower step (Theorem 131.T): {context}"
    )]
    UniverseAscentInvalid {
        /// Human-readable context (articulation name / call-site).
        context: Text,
        /// Source universe tier rendered as canonical text.
        from_tier: Text,
        /// Target universe tier rendered as canonical text.
        to_tier: Text,
    },

    /// V8 (#227) — `K-Coord-Cite` rule rejected a theorem
    /// citing an axiom whose `(Fw, ν, τ)` coordinate sits at a
    /// strictly higher ν tier.
    ///
    /// Per VVA §A.Z.5 item 2: a theorem at coordinate
    /// (Fw, ν, τ) may cite an axiom at coordinate (Fw', ν', τ')
    /// only when ν' ≤ ν (lex on [`crate::OrdinalDepth`]). Higher-
    /// tier citations are rejected unless the calling module
    /// imports the κ-tier-jump extension via
    /// `@require_extension(vfe_3)` (VVA-3 K-Universe-Ascent).
    ///
    /// The diagnostic carries both framework slugs + rendered
    /// ordinal-depth strings so the user can navigate the
    /// `(Fw, ν)` mismatch precisely.
    #[error(
        "kernel: K-Coord-Cite violation: theorem at \
         ('{theorem_fw}', ν={theorem_nu}) cites axiom \
         '{axiom_name}' at ('{axiom_fw}', ν={axiom_nu}) — \
         axiom's ν exceeds theorem's ν. Use \
         @require_extension(vfe_3) for κ-tier-jump."
    )]
    CoordViolation {
        /// Name of the cited axiom (registry key).
        axiom_name: Text,
        /// Framework of the citing theorem.
        theorem_fw: Text,
        /// Rendered ν of the citing theorem.
        theorem_nu: Text,
        /// Framework of the cited axiom.
        axiom_fw: Text,
        /// Rendered ν of the cited axiom.
        axiom_nu: Text,
    },

    /// V8 — `K-FwAx` body-is-Prop premise violated.
    ///
    /// Per `verification-architecture.md` §4.4, the K-FwAx rule
    /// has TWO independent soundness premises:
    ///
    ///   1. `body : Prop` — the axiom asserts a *proposition*, not
    ///      a non-trivial inhabitant of some `Type_n`. A
    ///      framework axiom of type `Π A B. A → B` would let users
    ///      postulate an arbitrary computable function and break
    ///      strong normalisation; restricting bodies to `Prop`
    ///      keeps the postulate at the propositional layer where
    ///      SN is preserved by the standard "axioms-stuck"
    ///      reduction strategy.
    ///   2. `body` is a subsingleton (closed proposition or UIP
    ///      regime) — see [`Self::AxiomNotSubsingleton`].
    ///
    /// V8 #217 shipped (2) but pre-V8 the kernel never enforced
    /// (1) at register time. This variant fires when an axiom's
    /// declared type, viewed as a CoreTerm via the empty Context,
    /// does NOT inhabit `Universe(Prop)` (or `Universe(Concrete(0))`
    /// under the set-theoretic reading where `Prop ⊆ Type_0`).
    ///
    /// `inferred_universe_shape` carries a coarse rendering of the
    /// universe the body actually inhabited (e.g.
    /// `"Concrete(2)"`) so the diagnostic message names which
    /// universe the body lives in.
    #[error(
        "kernel: framework axiom '{name}' body is not a Prop: \
         inferred universe shape is '{inferred_universe_shape}'; \
         framework axioms must inhabit Prop (or Type_0 in the \
         set-theoretic interpretation) to preserve strong \
         normalisation per §4.4 K-FwAx soundness premise"
    )]
    AxiomBodyNotProp {
        /// Axiom name being registered.
        name: Text,
        /// Coarse render of the inferred universe (e.g.
        /// `"Concrete(2)"`, `"Pi"`, `"Prop"`).
        inferred_universe_shape: Text,
    },

    /// V8 — `K-FwAx` subsingleton requirement violated. Per
    /// `verification-architecture.md` §4.4, a framework axiom's
    /// body must be a *subsingleton* (proof-irrelevant: at most
    /// one inhabitant up to definitional equality) for subject
    /// reduction to hold. Two acceptance routes:
    ///
    ///   1. **Closed-proposition route** — body mentions no free
    ///      type-variables. Closed Props are forced unique by the
    ///      framework lineage's intended interpretation.
    ///   2. **UIP route** — body mentions free type-vars and the
    ///      module explicitly imports
    ///      `core.math.frameworks.uip`. (Mixing UIP with
    ///      `core.math.frameworks.univalence` is rejected by
    ///      `framework_compat::audit_framework_set`.)
    ///
    /// This variant fires when neither route admits the body —
    /// the body has free vars AND the calling regime is
    /// [`crate::SubsingletonRegime::ClosedPropositionOnly`]. The
    /// `free_vars` field carries the offending names so the
    /// diagnostic identifies precisely which symbols escape the
    /// closed-proposition condition.
    ///
    /// Pre-V8 the kernel only checked the UIP-shape syntactically
    /// (rejecting `Π A. ∀ a b p q. p = q` via `UipForbidden`).
    /// That catches one specific paradox but admits a wide class
    /// of non-subsingleton axioms (e.g.
    /// `axiom choice<T>: ∀(s: NonEmpty<T>). T` whose witness depends
    /// on which element of `s` was chosen — non-canonical).
    /// V8 closes that gap by enforcing the full
    /// closed-proposition condition.
    #[error(
        "kernel: framework axiom '{name}' is not subsingleton: \
         body mentions {free_vars_count} free type-variable(s) \
         ({free_vars_rendered}); admit either via the closed-proposition \
         route (body must reference no unbound symbols) or via the \
         UIP route (module must @import core.math.frameworks.uip)"
    )]
    AxiomNotSubsingleton {
        /// Axiom name being registered.
        name: Text,
        /// Number of distinct free type-variables found.
        free_vars_count: usize,
        /// Comma-separated, sorted list of free-var names for the
        /// diagnostic message.
        free_vars_rendered: Text,
    },

    /// V8 — SmtProof obligation-hash mismatch. The certificate's
    /// declared `obligation_hash` does not equal the
    /// caller-supplied expected hash, so the certificate cannot be
    /// admitted as a proof of the caller's goal.
    ///
    /// Pre-V8 the kernel only checked that `obligation_hash` was
    /// non-empty (per `MissingObligationHash`); the doc comment on
    /// `replay_smt_cert` claimed "still checked against the
    /// caller's expected hash" but no such check existed. This
    /// allowed a certificate proving `X` to be re-used as a proof
    /// of `Y` if the user wrote `SmtProof(cert_for_X)` in a goal-Y
    /// context — soundness-fatal under the trust contract that
    /// puts the SMT backend OUTSIDE the TCB.
    ///
    /// V8 ships [`crate::support::replay_smt_cert_with_obligation`]
    /// which threads the expected hash through the replay and
    /// emits this variant on mismatch. The original
    /// [`crate::support::replay_smt_cert`] is preserved for kernel-
    /// internal callers that don't yet have the expected hash
    /// (e.g., the `infer` arm for `SmtProof` doesn't have a goal
    /// at type-inference time — the comparison happens at
    /// `verify_full`-style entry points instead).
    #[error(
        "kernel: SMT certificate obligation_hash mismatch — \
         expected '{expected}', certificate carries '{actual}'"
    )]
    ObligationHashMismatch {
        /// Hash the caller asserted the certificate must match.
        expected: Text,
        /// Hash actually present on the certificate.
        actual: Text,
    },

    /// V8 (#207, B1) — `K-Univ` universe-level overflow. The kernel's
    /// finite universe levels are encoded as `u32`; a request to
    /// type `Universe(Concrete(u32::MAX))` cannot honestly produce
    /// `Universe(Concrete(u32::MAX + 1))` and pre-V8
    /// `saturating_add(1)` silently returned `u32::MAX` again,
    /// producing the type-in-type rule `Universe(Concrete(u32::MAX))
    /// : Universe(Concrete(u32::MAX))` — soundness-fatal.
    ///
    /// Fix mirrors the B4 OrdinalDepth saturation hole: detect the
    /// overflow point and reject explicitly. Real Verum code uses
    /// universe levels in single digits (typical max is 2 or 3),
    /// so reaching `u32::MAX` in any honest workload is itself a
    /// strong indicator of an elaborator bug.
    ///
    /// Spec: `verification-architecture.md` §6.1 K-Univ rule;
    /// trusted-kernel.md rule 18 `Universe-Cumul` notes the
    /// implicit predicative-hierarchy invariant violated here.
    #[error(
        "kernel: K-Univ universe-level overflow at Concrete({level}); \
         cannot honestly produce successor (would silently saturate to \
         the same level, yielding type-in-type)"
    )]
    UniverseLevelOverflow {
        /// The level at which the overflow occurred (always
        /// `u32::MAX` today; field kept open for future encodings).
        level: u32,
    },
}
