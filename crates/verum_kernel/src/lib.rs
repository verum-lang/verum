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

/// Verum Unified Verification Architecture (VVA) version stamp.
///
/// Closes B14 . governance promises *"Каждое verification spec
/// принятие — minor version bump VVA"*; without a constant in code,
/// the version policy was unobservable. Tooling (CLI, certificate
/// emitters, cross-tool replay matrix per task #90) keys behaviour
/// on this constant.
///
/// **Bump policy** (per versioning):
///
///   * Major bump (`X` → `X+1`): backwards-incompatible changes to
///     [`CoreTerm`], [`KernelError`], or any `pub` kernel surface.
///   * Minor bump (`X.Y` → `X.Y+1`): verification spec kernel-rule acceptance,
///     or any new optional `@require_extension` gating.
///   * Patch bump (`X.Y.Z` → `X.Y.Z+1`): bug fixes, soundness
///     tightening (e.g., the B4 saturation fix in commit 3b15c185),
///     refactoring without API change.
///
/// Current version reflects the V0/V1/V2 K-Eps-Mu rule + V1
/// K-Universe-Ascent rule + V0/V1 K-Refine-omega rule shipped
/// B-series soundness
/// fixes. Bump on every kernel-rule addition.
pub const VVA_VERSION: &str = "2.6.0";

pub mod proof_checker;
// Note: `proof_checker::{check, infer, Context, Term}` deliberately
// stay un-reexported at crate root — the names collide with
// kernel-side `verum_kernel::infer` (the larger Verum-AST type
// inferencer) and the higher-level `Term`/`Context` carriers in
// `verum_kernel::term`.  The proof-term checker is the minimal
// trusted base; downstream callers reach it via the explicit
// `proof_checker::*` path so the namespace boundary stays loud.

pub mod tactic_elaborator;
// Tactic-to-proof-term elaboration (#164 Phase-1).  The missing link
// from Verum proof bodies (`ProofBody::Tactic(TacticExpr)`) to
// kernel-checkable [`proof_checker::Certificate`] values.  Without
// this module, the kernel checker is a *theoretically* trusted base
// but *practically* unused — no real Verum theorem reduces to a
// kernel-readable term.  The de Bruijn criterion lives here.

pub mod proof_tree;
pub use proof_tree::{KernelProofNode, KernelRule, record_inference};

/// Kernel error type — split into its own module for
/// auditability of the trusted-base diagnostic surface. Re-exported
/// at crate root so external callers see the pre-split path
/// `verum_kernel::KernelError` unchanged.
pub mod errors;
pub use errors::KernelError;

/// Inductive-type registry + strict-positivity checking. Hosts
/// `InductiveRegistry`, `RegisteredInductive`, `ConstructorSig`,
/// `PositivityCtx`, `check_strict_positivity` (K-Pos rule), plus the
/// UIP-shape detection helpers used by AxiomRegistry.
pub mod inductive;
pub use inductive::{
    ConstructorSig, InductiveRegistry, PathCtorSig, PositivityCtx,
    RegisteredInductive, check_strict_positivity, eliminator_type,
    point_constructor_case_type,
};

/// Depth functions for kernel rules — split . Hosts
/// `m_depth` (finite M-iteration depth, T-2f*), `m_depth_omega`
/// (ordinal modal-depth, T-2f***), `OrdinalDepth`, `check_refine_omega`
/// (K-Refine-omega rule entry point).
pub mod depth;
pub use depth::{OrdinalDepth, check_refine_omega, m_depth, m_depth_omega};

/// K-Eps-Mu kernel rule — split . Hosts
/// `check_eps_mu_coherence` with V0/V1/V2 staging.
pub mod eps_mu;
pub use eps_mu::{check_eps_mu_coherence, check_eps_mu_coherence_v3_final};

/// Categorical-coherence K-Universe-Ascent kernel rule + UniverseTier.
/// Hosts `UniverseTier` enum + `check_universe_ascent`.
pub mod universe_ascent;
pub use universe_ascent::{UniverseTier, check_universe_ascent};

/// `K-Round-Trip` kernel rule (V0/V1/V2) — OC/DC translation round-trip
/// admission for the AC/OC duality (MSFS Theorem 10.4 / Diakrisis
/// 108.T / 16.10). Hosts `check_round_trip` covering identity
/// (structural), K-Adj-Unit/Counit shapes, and β-/ι-/δ-equivalence
/// cases. V2 `check_round_trip_v2` ships the universal canonicalize
/// algorithm with explicit Diakrisis-16.10 bridge admits surfaced
/// via `BridgeAudit`.
pub mod round_trip;
pub use round_trip::{canonical_form, check_round_trip, check_round_trip_v2};

/// Diakrisis bridge admits — explicit, named axioms surfacing the
/// type-theoretic results currently outside the kernel's decidable
/// fragment. Each admit names a specific Diakrisis preprint result
/// (paragraph + theorem number) and is consumed by `K-Round-Trip V2`
/// to make preprint dependencies explicit at the kernel surface.
/// V3 promotion removes admits as the preprint resolves.
pub mod diakrisis_bridge;
pub use diakrisis_bridge::{BridgeAdmit, BridgeAudit, BridgeId};

pub use universe_ascent::{KappaTier, check_universe_ascent_v2};

/// Cubical cofibration calculus — face-formula algebra + interval
/// subsumption decision procedure (M-VVA-FU Sub-2.4-cubical, V1
/// shipped 2026-04-28). Per VVA spec L579 the cubical cofibration
/// calculus was deferred; this module provides:
///   * `FaceLit` — atomic literal `(i = 0)` / `(i = 1)`.
///   * `Clause` — DNF clause (conjunction of literals).
///   * `FaceFormula` — full DNF with ⊤/⊥/AND/OR + decidable
///     `implies` (subsumption via per-clause set inclusion).
/// Wired into HComp / Transp / Glue rules in `infer.rs` for
/// cofibration-coherence checking.
pub mod cofibration;
pub use cofibration::{Clause, FaceFormula, FaceLit};

/// Native ordinal arithmetic — Cantor normal form + inaccessible
/// cardinals + countable suprema.  Replaces ad-hoc `Int` placeholders
/// (999_999 = ω-1 etc.) used pre-this-module.  Supports decidable
/// `lt` / `succ` / `is_regular` / `is_limit` / `is_inaccessible` on
/// the Cantor-normal-form fragment + κ-tower; `Sup` of countable
/// family for ordinals beyond Cantor normal form (`ε_0` and above).
pub mod ordinal;
pub use ordinal::Ordinal;

/// Native (∞,n)-categorical kernel infrastructure (V0).  No mainstream
/// proof assistant carries first-class ∞-categorical reasoning in
/// its kernel; this is Verum's novel contribution.  Ships:
///
///   * `InfinityCategory` / `InfinityMorphism` / `InfinityEquivalence`
///     — native CoreTerm-adjacent representations.
///   * `identity_is_equivalence(x, n)` — the fundamental kernel
///     rule that `id_X` is an (∞,n)-equivalence for every `n: Ordinal`.
///     Discharges MSFS Theorem 5.1's id_X-violates-Π_4 step in-kernel
///     for every concrete level.
///   * `is_equivalence_at(f, n, audit, ctx)` — V0 equivalence-decision
///     rule with explicit `BridgeAudit` for limit-level / inaccessible
///     cases.
///   * `compose(f, g)` + `compose_is_associative(f, g, h)` — native
///     composition with strict associativity at level 1.
///
/// V1+ promotion paths documented in module-level docs.
pub mod infinity_category;
pub use infinity_category::{
    CellLevel, InfinityCategory, InfinityEquivalence, InfinityMorphism,
    compose, compose_is_associative, identity_is_equivalence, is_equivalence_at,
};

/// HTT 5.1.4 ∞-Grothendieck construction — V0 algorithmic kernel rule.
/// The load-bearing technical pivot for MSFS Lemma 3.4 (and AFN-T).
/// Pre-this-module the construction was admitted as
/// `lurie_htt_5_1_4_syn_is_grothendieck` framework axiom; V0 ships an
/// algorithmic builder that constructively produces the total
/// Cartesian fibration `∫D = { (b, x) : b ∈ B, x ∈ D(b) }` from any
/// `S`-indexed diagram, with explicit accessibility-preservation
/// witness per AR 1.26.
///
/// V1 promotion: full ∞-categorical higher-cell content (associator
/// 2-cells, pentagonal coherence) — gates Theorem 9.3 Step 1.
pub mod grothendieck;
pub use grothendieck::{
    GrothendieckConstruction, SIndexedDiagram, build_grothendieck,
};

/// Adámek-Rosický 1.26 — λ-filtered colimit closure of κ-accessible
/// categories.  V0 algorithmic kernel rule.  Discharges the
/// "κ_1-accessibility preserved under transfinite-tower colimit"
/// invariant that gates MSFS §6 β-part Step 4.  Pre-this-module
/// admitted via msfs_lemma_A_8_adamek_rosicky framework axiom;
/// V0 ships the constructive closure operation itself.
pub mod accessibility;
pub use accessibility::{
    FilteredColimit, KappaAccessibleCategory, LambdaFilteredDiagram,
    build_filtered_colimit, cofinality_bound_holds,
};

/// Yoneda embedding (HTT 1.2.1) + ∞-Kan extensions (HTT 4.3.3.7) —
/// V0 algorithmic kernel rules.  Gates MSFS Definition 3.3 (S_S
/// closure under Yoneda + Kan-extension along S-definable morphisms).
///
/// Pre-this-module the Yoneda closure was admitted via the host
/// stdlib axiom `msfs_s_s_closed_under_yoneda` and Kan-extension
/// closure routed through O1 of the same definition.  V0 ships:
///
///   * `Presheaf` / `YonedaEmbedding` / `YonedaLemma` — first-class
///     ∞-categorical surface representations.
///   * `yoneda_embedding(c)` — fully-faithful embedding witness with
///     fullness-level certification (HTT 1.2.1).
///   * `presheaf_category(c)` — `PSh(C)` builder with universe ascent
///     (HTT 5.5).
///   * `build_kan_extension(...)` — left Kan extension `Lan_f(p)`
///     under fully-faithful-along-functor + target-has-colimits
///     preconditions (HTT 4.3.3.7).
///   * `kan_extension_unit_witness` — universal-property witness.
///
/// V1 promotion: full higher-cell content (associator + pentagonal
/// coherence cells).
pub mod yoneda;
pub use yoneda::{
    KanExtension, Presheaf, YonedaEmbedding, YonedaLemma,
    build_kan_extension, kan_extension_unit_witness,
    presheaf_category, yoneda_embedding, yoneda_lemma,
};

/// Cartesian fibrations (HTT 3.1) + Straightening/Unstraightening
/// equivalence (HTT 3.2.0.1) — V0 algorithmic kernel rules.  Gates
/// MSFS Theorem 9.3 Step 1 (currently admits via host-stdlib axiom
/// `msfs_htt_3_2_straightening`) and §6 β-part Step 2.
///
/// Ships:
///   * `CartesianFibration` / `CartesianMorphism` — first-class
///     ∞-categorical surface representations.
///   * `is_cartesian(p, f)` — decision predicate for p-Cartesian
///     morphisms (HTT 3.1.1.1).
///   * `StraighteningEquivalence` — witness for
///     `St : coCart(C) ≃ Fun(C, ∞-Cat) : Un` (HTT 3.2.0.1).
///   * `build_straightening_equivalence(c)` — algorithmic builder.
///   * `unstraighten_to_grothendieck` — bridge identifying `Un`
///     with `crate::grothendieck::build_grothendieck`.
///   * `fibration_is_unstraightened` — recognise fibrations arising
///     via unstraightening.
///
/// V1 promotion: full higher-cell coherence content (associator
/// 2-cells + pentagonal coherence between St and Un).
pub mod cartesian_fibration;
pub use cartesian_fibration::{
    CartesianFibration, CartesianMorphism, StraighteningEquivalence,
    build_straightening_equivalence, fibration_is_unstraightened,
    is_cartesian, unstraighten_to_grothendieck,
};

/// Adjoint Functor Theorem (HTT 5.5.2.9 / Special AFT) — V0
/// algorithmic kernel rule.  Gates MSFS Lemma 10.3 (the (ι, r)
/// reflective subcategory construction) and Diakrisis 16.3.
///
/// Ships:
///   * `Adjunction` — first-class adjoint-pair representation with
///     unit/counit/triangle-identities witnesses.
///   * `SaftPreconditions` — HTT 5.5.2.9 input precondition record.
///   * `left_adjoint_exists(pre)` / `right_adjoint_exists(pre)` —
///     decidable predicates per HTT 5.5.2.9.
///   * `build_adjunction(...)` — algorithmic builder under SAFT
///     preconditions.
///   * `compose_adjunctions(first, second)` — adjunction
///     composition (the 2-categorical monoidal structure).
///   * `triangle_identities_witness` — universal-property witness.
///
/// V1 promotion: explicit unit/counit natural-transformation cells
/// with full pentagonal coherence between composition and identity.
pub mod adjoint_functor;
pub use adjoint_functor::{
    Adjunction, AdjunctionDirection, SaftPreconditions,
    build_adjunction, compose_adjunctions, left_adjoint_exists,
    right_adjoint_exists, triangle_identities_witness,
};

/// Whitehead criterion for (∞, n)-equivalence (HTT 1.2.4.3
/// generalised) — V0 algorithmic kernel rule.  The decidable
/// characterisation of equivalence via per-level homotopy-group
/// iso witnesses.  **Trusted-base-shrinkage primitive**:
/// Whitehead-certified equivalences carry empty `BridgeAudit`,
/// shrinking the surface visible to `verum audit --proof-honesty`.
///
/// Ships:
///   * `PiLevelIso` — per-level `π_k` iso witness.
///   * `WhiteheadCriterion` — full per-level certificate with
///     `identity_at` constructor for trivial cases.
///   * `is_equivalence_via_whitehead(criterion)` — bridge-free
///     decision predicate.
///   * `whitehead_promote(criterion, audit)` — promote to
///     `InfinityEquivalence` with empty audit.
///   * `KanComplexLift` + `weak_equivalence_lifts_in_kan_complex` —
///     HTT 1.2.4.3 (weak ⟹ honest equivalence in a Kan complex).
///
/// V1 promotion: structural inspection of each iso witness instead
/// of trust-then-verify on the witness flag.
pub mod whitehead;
pub use whitehead::{
    KanComplexLift, PiLevelIso, WhiteheadCriterion,
    is_equivalence_via_whitehead, weak_equivalence_lifts_in_kan_complex,
    whitehead_promote,
};

/// Reflective subcategories (HTT 5.2.7) — V0 algorithmic kernel rule.
/// Composes [`adjoint_functor`] (SAFT) with idempotency to formalise
/// "reflective subcategory" as a first-class concept.  Gates MSFS
/// Lemma 10.3 + Diakrisis 16.3 fully (host-stdlib `msfs_aft_iota_r`
/// admit can be promoted to direct invocation).
///
/// Ships:
///   * `ReflectiveSubcategory` — first-class record `(D, C, ι, r, η)`
///     with fully-faithful + adjunction + idempotency witnesses.
///   * `is_reflective(rs)` — decidable predicate per HTT 5.2.7.2.
///   * `build_reflective_subcategory(...)` — algorithmic builder
///     under HTT 5.2.7.4 preconditions.
///   * `idempotency_witness` / `reflector_unit_is_localisation` —
///     universal-property witnesses (HTT 5.2.7.4 (iii)/(iv)).
///
/// V1 promotion: explicit unit / idempotency natural-transformation
/// cells with full pentagonal coherence.
pub mod reflective_subcategory;
pub use reflective_subcategory::{
    ReflectiveSubcategory, build_reflective_subcategory,
    idempotency_witness, is_reflective, reflector_unit_is_localisation,
};

/// Limits and colimits in (∞,1)-categories — V0 algorithmic kernel
/// rule (HTT 1.2.13 + HTT 5.5.3 + HTT 4.4).  Gates MSFS Definition
/// 3.3 closure under (co)limits (replaces `msfs_s_s_closed_under_colimits`
/// host-stdlib admit).
///
/// Ships:
///   * `LimitShape` — coarse shape classification (Terminal / Pullback
///     / Equaliser / Filtered / Small).
///   * `LimitDiagram` / `ColimitDiagram` — diagram input data.
///   * `Limit` / `Colimit` — output records with universal-cone /
///     -cocone witnesses.
///   * `presheaf_admits_limits` / `presheaf_admits_colimits` —
///     decision predicates per HTT 5.5.3.5.
///   * `compute_limit_in_psh` / `compute_colimit_in_psh` —
///     pointwise (HTT 5.1.2.3) algorithmic builders.
///   * Specialised constructors: `build_pullback`, `build_pushout`,
///     `build_equaliser`, `build_coequaliser`, `build_terminal`,
///     `build_initial`.
///   * `promote_limit_to_level` — level-monotonicity promotion.
///   * `presheaf_is_bicomplete(c)` — HTT 5.5.3.5 witness.
///
/// V1 promotion: explicit universal-cone natural transformations
/// with full pentagonal coherence cells.
pub mod limits_colimits;
pub use limits_colimits::{
    Colimit, ColimitDiagram, Limit, LimitDiagram, LimitShape,
    build_coequaliser, build_equaliser, build_initial, build_pullback,
    build_pushout, build_terminal, colimit_universal_property,
    compute_colimit_in_psh, compute_limit_in_psh,
    limit_universal_property, presheaf_admits_colimits,
    presheaf_admits_limits, presheaf_is_bicomplete,
    promote_limit_to_level,
};

/// n-truncation operators for (∞,1)-categories — V0 algorithmic
/// kernel rule (HTT 5.5.6).  The level-descent operator
/// `τ_{≤n}: C → C_{≤n}` quotienting (n+1)-cells and higher.
///
/// Ships:
///   * `Truncation` — the apex of `τ_{≤n}(x)` with universal-property
///     witness.
///   * `truncate_to_level(x, c, n)` — algorithmic builder.
///   * `is_n_truncated` — decidable predicate per HTT 5.5.6.1.
///   * `truncation_unit_witness` — universal-property cone for
///     `η : x → τ_{≤n}(x)`.
///   * `truncation_is_localisation` — HTT 5.5.6.18 witness.
///   * `truncation_left_adjoint_to_inclusion` — HTT 5.5.6.21 witness.
///   * `n_truncated_objects_closed_under_limits` — HTT 5.5.6.5
///     unconditional theorem.
///   * `compose_truncations(outer, inner)` — level-descent
///     composition collapsing to `min(m, n)`.
///
/// V1 promotion: explicit unit / counit cells with structural
/// level-descent traces.
pub mod truncation;
pub use truncation::{
    Truncation, compose_truncations, is_n_truncated,
    n_truncated_objects_closed_under_limits, truncate_to_level,
    truncation_is_localisation, truncation_left_adjoint_to_inclusion,
    truncation_unit_witness,
};

/// Factorisation systems on (∞,1)-categories — V0 algorithmic
/// kernel rule (HTT 5.2.8).  Orthogonal `(L, R)` pairs where every
/// morphism factors as `f = r ∘ l`.  Gates MSFS §6 β-part Step 5
/// (replaces `msfs_epi_mono_factorisation` admit) and §9 Theorem 9.3
/// Step 4 (replaces `msfs_n_truncation_factorisation` admit).
///
/// Ships:
///   * `FactorisationSystem` — the `(L, R)` data with closure +
///     orthogonality + factorisation witnesses.
///   * `Factorisation` — concrete `f = r ∘ l` decomposition.
///   * `is_orthogonal(fs)` — decision predicate per HTT 5.2.8.5.
///   * `factorise(fs, f)` — algorithmic builder.
///   * `factorisation_uniqueness` — HTT 5.2.8.4 uniqueness witness.
///   * `closure_under_composition` — HTT 5.2.8.6 witness.
///   * Specialised constructors:
///     - `build_epi_mono_factorisation` (HTT 5.2.8.4).
///     - `build_n_truncation_factorisation` (HTT 5.2.8.16) —
///       composes with [`crate::truncation`].
///     - `build_localisation_factorisation` (HTT 5.2.7.5) —
///       composes with [`crate::reflective_subcategory`].
///
/// V1 promotion: explicit lifting cells with full pentagonal
/// coherence between orthogonality and factorisation.
pub mod factorisation;
pub use factorisation::{
    Factorisation, FactorisationSystem, build_epi_mono_factorisation,
    build_localisation_factorisation, build_n_truncation_factorisation,
    closure_under_composition, factorisation_uniqueness, factorise,
    is_orthogonal,
};

/// Pronk's bicategory of fractions — V0 algorithmic kernel rule
/// (Pronk 1996).  Constructs `C[W^{-1}]` as a bicategory under the
/// BF1–BF5 axioms.  Gates Diakrisis 16.10 (the AC/OC duality
/// classifier — currently admits via `diakrisis_pronk_bicat_fractions`)
/// and MSFS Theorem 9.3 Step 3.
///
/// Ships:
///   * `PronkAxioms` — BF1–BF5 axiom-witness record with
///     `fully_satisfied()` constructor.
///   * `Span` — span data carrier `X ←w Y' → Y` representing a
///     morphism in `C[W^{-1}]`.
///   * `BicatOfFractions` — the resulting bicategory with
///     universal-functor witness.
///   * `build_bicat_of_fractions(c, w, axioms)` — algorithmic builder.
///   * `compose_spans(first, second)` — span composition via Ore-pullback.
///   * `universal_2_functor(bicat)` — universal-property witness.
///
/// V1 promotion: explicit pentagonal coherence cells for span
/// composition + full bicategorical 2-cell content.
pub mod pronk_fractions;
pub use pronk_fractions::{
    BicatOfFractions, PronkAxioms, Span, build_bicat_of_fractions,
    compose_spans, universal_2_functor,
};

/// (∞,1)-topos infrastructure — V0 algorithmic kernel rule
/// (Lurie HTT 6.1).  An (∞,1)-topos is a left-exact localisation of
/// a presheaf ∞-category satisfying Giraud's axioms (HTT 6.1.0.4).
/// **Caps the foundational ∞-cat layer**: composes
/// [`reflective_subcategory`] + [`limits_colimits`] +
/// left-exactness witness.
///
/// Ships:
///   * `GiraudAxioms` — the four Giraud-axiom witnesses
///     (presentable, universal-colimits, disjoint-coproducts,
///     effective-groupoids).
///   * `InfinityTopos` — first-class topos record.
///   * `is_infinity_topos(t)` — decidable predicate per
///     HTT 6.1.0.4.
///   * `build_infinity_topos(...)` — algorithmic builder under
///     HTT 6.1.0.6 preconditions.
///   * `presheaf_category_is_topos(c)` — HTT 6.1.0.6 (i):
///     every `PSh(C)` is canonically an (∞,1)-topos.
///   * `left_exact_localisation_witness` — HTT 6.1.0.6 (ii)
///     witness flag.
///
/// Discharges MSFS §3 admit `msfs_s_s_is_infty_topos`.
///
/// V1 promotion: structural checking of effective-groupoid +
/// universal-colimit content (the V0 surface trusts the witness flags).
pub mod infinity_topos;
pub use infinity_topos::{
    GiraudAxioms, InfinityTopos, build_infinity_topos,
    is_infinity_topos, left_exact_localisation_witness,
    presheaf_category_is_topos,
};

/// Kernel self-recognition vs. ZFC + 2 inaccessibles — V0
/// algorithmic kernel rule.  The relative-consistency surface that
/// decomposes each of the seven kernel rules (K-Refine, K-Univ,
/// K-Pos, K-Norm, K-FwAx, K-Adj-Unit, K-Adj-Counit) into the precise
/// ZFC-axiom + Grothendieck-universe requirements needed to model it.
///
/// Ships:
///   * `ZfcAxiom` enumeration (all 9 axioms incl. Choice).
///   * `InaccessibleLevel` (Kappa1 / Kappa2).
///   * `KernelRuleId` (the seven rules).
///   * `MetaTheoryRequirements` — per-rule decomposition.
///   * `required_meta_theory(rule)` — algorithmic decomposition.
///   * `is_zfc_plus_2_inacc_provable(rule)` — decision predicate.
///   * `SelfRecognitionAudit` — accumulator with `cite`/`report` and
///     transitive ZFC-axiom + inaccessibles union queries.
///
/// **Self-recognition invariant**: every kernel rule is provable in
/// ZFC + 2 inaccessibles.  Discharges VVA §16.5 Phase 5 audit
/// surface.
pub mod zfc_self_recognition;
pub use zfc_self_recognition::{
    InaccessibleLevel, KernelRuleId, MetaTheoryRequirements,
    SelfRecognitionAudit, ZfcAxiom, is_zfc_plus_2_inacc_provable,
    required_meta_theory,
};

/// Recursive functions + Gödel coding — V0 algorithmic kernel rule.
/// Provides the decidable encoding-of-formulae machinery required for
/// Gödel-style incompleteness arguments and Yanofsky 2003 diagonal
/// paradox claims.
///
/// Ships:
///   * `PrimitiveRecursive` — Kleene normal form (Zero / Succ / Proj
///     / Comp / PrimRec) with totally-evaluating `eval`.
///   * `MuRecursive` — extends primitive recursion with bounded
///     `MuMin` minimisation.
///   * `cantor_pair` / `cantor_unpair` — bijection ℕ × ℕ → ℕ.
///   * `encode_list` / `decode_list` — list-of-symbols ↔ Gödel number.
///   * `GodelEncoding` — recursive AST cell with `encode` to u64.
///   * `is_primitive_recursive` / `is_mu_recursive` /
///     `representable_in_pa` — class-membership predicates.
///
/// V1 promotion: full kernel-CoreTerm round-trip via [`encode_term`] /
/// [`decode_term`] (V0 ships symbol-level cells).
pub mod godel_coding;
pub use godel_coding::{
    GodelEncoding, MuRecursive, PrimitiveRecursive,
    cantor_pair, cantor_unpair, decode_list, encode_list,
    is_mu_recursive, is_primitive_recursive, representable_in_pa,
};

/// Industrial-grade tactic infrastructure — V0 algorithmic kernel
/// rule.  Production tactics that close subgoals via deterministic
/// decision procedures (no SMT delegation).  Five built-in tactics:
///
///   * `tactic_lia` — Linear Integer Arithmetic (V0 surface: trivial
///     constraints; V1 promotes to Omega-test).
///   * `tactic_decide` — boolean tautology decision via truth-table
///     exhaustion (≤ 16 atoms; V1 promotes to BDD/SAT).
///   * `tactic_induction` — ℕ-induction split (`P(0) ∧ ∀k. P(k) ⇒
///     P(k+1)` ⇒ `∀n. P(n)`).
///   * `tactic_congruence` — EUF equality closure via union-find.
///   * `tactic_eauto` — bounded back-chaining over hint database.
///
/// Each returns a `TacticOutcome::{Closed, Open}` carrying a
/// re-checkable witness; the kernel re-checks the witness in linear
/// time relative to its size.
pub mod tactics_industrial;
pub use tactics_industrial::{
    BoolFormula, CongruenceEquation, EautoHint, InductionSplit,
    LinearConstraint, LinearRelation, TacticOutcome,
    tactic_congruence, tactic_decide, tactic_eauto, tactic_induction,
    tactic_lia,
};

/// Cross-format CI hard gate — V0 algorithmic kernel rule.  Decides
/// whether a given proof artefact survives the round-trip through
/// every required foreign proof-assistant backend (Coq, Lean 4,
/// Isabelle/HOL, Dedukti).  Used by `verum audit --cross-format`.
pub mod cross_format_gate;
pub use cross_format_gate::{
    CrossFormatReport, ExportFormat, FormatStatus,
    evaluate_gate, format_replay_command, required_formats_for_msfs,
};

/// Mechanisation roadmap — V0 algorithmic kernel surface enumerating
/// HTT (Lurie 2009) + Adámek-Rosický 1994 chapter-by-chapter
/// mechanisation status.  Used by `verum audit --htt-roadmap` and
/// `verum audit --adamek-rosicky-roadmap` for structured coverage
/// queries comparable across releases.
pub mod mechanisation_roadmap;
pub use mechanisation_roadmap::{
    CoverageReport, MechanisationStatus, RoadmapEntry,
    adamek_rosicky_roadmap, htt_roadmap,
    msfs_self_contained, msfs_unmechanised_dependencies,
};

/// Kernel intrinsic dispatch — string-name → kernel-function bridge.
/// The single uniform entry point that downstream callers (compiler
/// elaborator, audit tooling, proof-body verifier) use to invoke a
/// kernel intrinsic by its `kernel_*` name with a typed argument
/// list.  Decouples the 15-module kernel surface from any one
/// caller's invocation convention.
pub mod intrinsic_dispatch;
pub use intrinsic_dispatch::{
    IntrinsicValue, available_intrinsics, dispatch_intrinsic,
    is_known_intrinsic, missing_dispatchers,
};

/// Meta-circular kernel soundness export — the cross-tool
/// re-checking surface for task #80 / VERUM-TRUST-1.  Walks the
/// 35-rule corpus declared in `core/verify/kernel_soundness/` and
/// emits parallel theory files for Coq + Lean (and any other
/// `SoundnessBackend` implementation).  See module docs for the
/// architectural rationale (one trait, multiple instances).
pub mod soundness;
pub use soundness::{
    EXPECTED_KERNEL_RULE_COUNT, LemmaStatus, RuleCategory, RuleSpec,
    SoundnessBackend, SoundnessExporter, canonical_rules,
    coq::CoqBackend, lean::LeanBackend,
};

/// Supporting kernel operations — `shape_of`, `substitute`,
/// `structural_eq`, `replay_smt_cert`. Split . The
/// kernel's "infrastructure layer": these don't implement a
/// typing rule themselves but every rule in `infer` / `check`
/// calls one or more of them.
pub mod support;
pub use support::{
    EpsInvariant, NORMALIZE_STEP_LIMIT, convert_eps_to_md_omega, definitional_eq,
    definitional_eq_with_axioms, free_vars, is_face_bot, is_face_top, is_interval_one,
    normalize, normalize_with_axioms, normalize_with_inductives, replay_smt_cert,
    replay_smt_cert_with_obligation, shape_of, structural_eq, substitute,
};

/// NormalizeCache (#100, task #42) — DashMap memo for normalize
/// results keyed on a stable structural hash of the input term.
/// Mirror of `verum_smt::tactics::TacticCache` for the kernel side.
pub mod normalize_cache;
pub use normalize_cache::{
    AxiomAwareKey, NormalizeCache, NormalizeCacheStats, StructuralHash,
};

/// Axiom registry + AST loader — split . Hosts
/// `AxiomRegistry`, `RegisteredAxiom`, `LoadAxiomsReport`, and
/// `load_framework_axioms`. UIP-shape axioms are syntactically
/// rejected to preserve cubical-univalence soundness.
pub mod axiom;
pub use axiom::{
    AxiomRegistry, LoadAxiomsReport, RegisteredAxiom, SubsingletonRegime,
    load_framework_axioms, load_framework_axioms_legacy_unchecked,
    load_framework_axioms_strict, load_framework_axioms_with_regime,
};

/// Kernel typing judgment — split . Hosts the core LCF
/// `infer` function plus the `check` / `verify` / `verify_full`
/// shells callers use to gate proof admission.
pub mod infer;
pub use infer::{
    check, infer, infer_with_full_context, infer_with_inductives, verify, verify_full,
};

/// Core syntactic surface — `CoreTerm`, `CoreType`, `UniverseLevel`.
/// Split V7. The explicit calculus the kernel checks; every
/// other kernel module builds on top of these three types.
pub mod term;
pub use term::{CoreTerm, CoreType, UniverseLevel};

/// SMT certificate envelope — `SmtCertificate` +
/// `CERTIFICATE_SCHEMA_VERSION`. Split V7.
pub mod cert;
pub use cert::{CERTIFICATE_SCHEMA_VERSION, SmtCertificate};

/// Typing context + framework-axiom attribution — `Context` +
/// `FrameworkId`. Split V7.
pub mod ctx;
pub use ctx::{Context, FrameworkId, KernelCoord, check_coord_cite};

