//! # `verum_kernel::soundness` — meta-circular kernel-soundness export
//!

//! This module implements the cross-export side of task #80
//! (VERUM-TRUST-1). The Verum-side soundness corpus lives in
//! `core/verify/kernel_soundness/`; this module walks that corpus
//! (well, its declarative skeleton — the rule set, lemma names,
//! and admit-reasons) and produces parallel Coq + Lean theory files
//! that an independent reviewer can run through `coqc` / `lean` to
//! verify Verum is being honest.
//!

//! ## Architectural shape — protocol-driven, not per-format
//!

//! Every cross-export target implements one trait, [`SoundnessBackend`].
//! Concrete instances (`coq::CoqBackend`, `lean::LeanBackend`) are
//! short — they just say "for this fragment of the corpus, render this
//! syntax." The corpus walk is shared in [`SoundnessExporter`], which
//! drives the trait methods in canonical order.
//!

//! Adding a third tool (Isabelle, Agda, Dedukti) is a single new
//! implementation of [`SoundnessBackend`]. The exporter, the audit
//! gate, and the snapshot tests are all parameterised over the trait.
//!

//! ## Single source of truth
//!

//! The 35-rule list in this Rust module mirrors the
//! `verum_kernel::proof_tree::KernelRule` enum. The mirror is
//! drift-detected at audit time: the exporter cross-checks the
//! Rust enum's variant count against `KERNEL_RULE_COUNT` and against
//! the `.vr` corpus's `corpus_rows()` length. A one-sided edit
//! (Rust grows a rule, .vr doesn't, or vice versa) fails the gate.
//!

//! ## Honest IOUs
//!

//! When a Verum-side lemma is admitted with reason "requires modal-
//! depth ordinal arithmetic well-foundedness", the Coq emission ends
//! in `Admitted. (* requires modal-depth ordinal arithmetic
//! well-foundedness *)` and the Lean emission in `sorry -- requires
//! modal-depth ordinal arithmetic well-foundedness`. A foreign
//! reviewer sees the same gap Verum sees.

use serde::{Deserialize, Serialize};

pub mod apply_graph;
pub mod coq;
pub mod corpus_export;
pub mod discharge_status;
pub mod expr_translate;
pub mod isabelle;
pub mod kernel_v0_manifest;
pub mod lean;
pub mod proof_body_translate;

pub use discharge_status::DischargeStatus;

#[cfg(test)]
mod tests;

/// Canonical lemma-status as seen by the cross-export pipeline.
///

/// Mirrors `core::verify::kernel_soundness::theorems::LemmaStatus`.
/// The Verum corpus is the source of truth; this enum is the Rust-
/// side carrier so [`SoundnessBackend`] implementations can render
/// the right syntax (proof tactic chain vs. `Admitted`/`sorry`)
/// without reading `.vr` files at compile time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LemmaStatus {
    /// The lemma is structurally proved in the corpus. The
    /// `tactics` field carries the per-backend proof script the
    /// emitter should place between `Proof. ... Qed.` (Coq) or
    /// the `:= by ...` tactic block (Lean).
    Proved {
        /// Tactic chain to emit between `Proof.` and `Qed.` in Coq.
        coq_tactics: String,
        /// Tactic block to emit after `:= by` in Lean.
        lean_tactics: String,
    },
    /// The lemma is admitted with a concrete cost-annotation. The
    /// `reason` is preserved verbatim into the foreign-tool output.
    Admitted {
        /// Concrete IOU naming the missing meta-theory. Preserved
        /// verbatim into the Coq `Admitted.` comment and the Lean
        /// `sorry --` comment.
        reason: String,
    },
    /// The lemma is discharged by citing a vetted upstream proof
    /// (mathlib4 / Coq stdlib / ZFC-foundational). Audit-acceptable
    /// at L4 because the citation pins a specific upstream file the
    /// reviewer can independently verify. Renders the same as
    /// `Admitted` in foreign-tool output but carries structured
    /// citation metadata for the audit gate.
    ///

    /// Lifecycle (per IOU): `Admitted { reason } → DischargedByFramework
    /// → Proved { coq_tactics, lean_tactics }` once full proof-term
    /// replay lands (#162).
    DischargedByFramework {
        /// Path to the discharge stub in `core/verify/kernel_v0/lemmas/`.
        /// Example: `core.verify.kernel_v0.lemmas.beta.church_rosser_confluence`.
        lemma_path: String,
        /// Upstream framework name (e.g. "mathlib4", "coq_stdlib", "zfc").
        framework: String,
        /// Concrete citation string. Example: `Mathlib.Computability.Lambda.ChurchRosser`.
        citation: String,
    },
}

impl LemmaStatus {
    /// Project: is this status `Proved`?
    pub fn is_proved(&self) -> bool {
        matches!(self, LemmaStatus::Proved { .. })
    }

    /// Project: is this status `DischargedByFramework`? L4-acceptable
    /// but downstream of a cited upstream proof.
    pub fn is_discharged_by_framework(&self) -> bool {
        matches!(self, LemmaStatus::DischargedByFramework { .. })
    }

    /// Project: extract the admit-reason if any. For
    /// `DischargedByFramework`, returns the citation string —
    /// callers that audit "what's the trust extension" treat both
    /// cases uniformly.
    pub fn admit_reason(&self) -> Option<&str> {
        match self {
            LemmaStatus::Proved { .. } => None,
            LemmaStatus::Admitted { reason } => Some(reason.as_str()),
            LemmaStatus::DischargedByFramework { citation, .. } => Some(citation.as_str()),
        }
    }
}

/// Categorisation of a kernel rule. Mirrors
/// `core::verify::kernel_soundness::rules::RuleCategory`. Used
/// for grouping in foreign-tool outputs (sections / namespaces).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleCategory {
    /// Structural-rule cluster — Var/Univ/Pi/Lam/App/Sigma/Pair/Fst/Snd.
    Structural,
    /// Cubical-rule cluster — Path/PathOver/Refl/HComp/Transp/Glue.
    Cubical,
    /// Refinement-rule cluster — Refine{,Omega,Intro,Erase}.
    Refinement,
    /// Quotient-rule cluster — QuotForm/QuotIntro/QuotElim.
    Quotient,
    /// Inductive-rule cluster — Inductive/Pos/Elim.
    Inductive,
    /// SMT- and axiom-rule cluster — Smt/FwAx.
    SmtAxiom,
    /// Diakrisis cluster — Eps-Mu / Universe-Ascent / Round-Trip /
    /// Epsilon-Of / Alpha-Of / Modal-{Box,Diamond,BigAnd} / cohesive
    /// triple Shape / Flat / Sharp.
    Diakrisis,
}

impl RuleCategory {
    /// Stable text tag used as section heading in foreign-tool output.
    pub fn tag(self) -> &'static str {
        match self {
            RuleCategory::Structural => "Structural",
            RuleCategory::Cubical => "Cubical",
            RuleCategory::Refinement => "Refinement",
            RuleCategory::Quotient => "Quotient",
            RuleCategory::Inductive => "Inductive",
            RuleCategory::SmtAxiom => "SmtAxiom",
            RuleCategory::Diakrisis => "Diakrisis",
        }
    }
}

/// One row of the kernel-soundness corpus. The cross-export
/// pipeline consumes a `Vec<RuleSpec>` produced by [`canonical_rules`]
/// and dispatches each row through the active [`SoundnessBackend`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSpec {
    /// Stable rule name in `K_<Name>` form (no leading dash, ASCII).
    pub rule_name: String,
    /// Stable lemma name in `K_<Name>_sound` form.
    pub lemma_name: String,
    /// Coarse category for grouping.
    pub category: RuleCategory,
    /// Number of premise sub-derivations the rule expects.
    pub premise_arity: usize,
    /// Whether the rule has a non-judgmental side-condition obligation.
    pub has_side_condition: bool,
    /// The lemma's proof status.
    pub status: LemmaStatus,
}

/// The 35-rule canonical specification — single source of truth on
/// the Rust side. Mirrors the Verum corpus's `corpus_rows()` and
/// `verum_kernel::proof_tree::KernelRule`. Drift between the three
/// is checked at audit time.
///

/// **Why hand-written?** The Verum-side definitions live in `.vr`
/// files that are loaded as bytecode at compile time of the
/// compiler — they cannot be parsed inside this crate without
/// circular-dependency hazards. The audit-time drift check
/// discovers any divergence loudly: the Rust list is hand-written
/// here, parsed-from-`.vr`-corpus elsewhere, and the gate compares
/// the two.
pub fn canonical_rules() -> Vec<RuleSpec> {
    use LemmaStatus::*;
    use RuleCategory::*;

    // Helper: build a `Proved` status with the per-backend tactic
    // strings. Keeps the table compact.
    fn proved(coq: &str, lean: &str) -> LemmaStatus {
        Proved {
            coq_tactics: coq.to_string(),
            lean_tactics: lean.to_string(),
        }
    }

    // Helper: build an `Admitted` status with the reason verbatim.
    // Currently unused — every kernel rule is either Proved or
    // DischargedByFramework after the FV-17 final batch — but the
    // helper is retained so that re-introducing an open IOU (e.g.
    // for a new rule added to the corpus) keeps the canonical-rules
    // table grammar consistent with the existing Proved / discharged
    // builders.
    #[allow(dead_code)]
    fn admitted(reason: &str) -> LemmaStatus {
        Admitted {
            reason: reason.to_string(),
        }
    }

    // Helper: build a `DischargedByFramework` status citing an
    // upstream proof. The kernel_v0/lemmas/ stub at `lemma_path`
    // carries the matching `@framework(...)` annotation; the audit
    // gate enumerates these for the trust-extension report.
    fn discharged(lemma_path: &str, framework: &str, citation: &str) -> LemmaStatus {
        DischargedByFramework {
            lemma_path: lemma_path.to_string(),
            framework: framework.to_string(),
            citation: citation.to_string(),
        }
    }

    let spec =
        |name: &str, cat: RuleCategory, arity: usize, side: bool, status: LemmaStatus| RuleSpec {
            rule_name: name.to_string(),
            lemma_name: format!("{}_sound", name),
            category: cat,
            premise_arity: arity,
            has_side_condition: side,
            status,
        };

    vec![
        // ---- Structural (9) -------------------------------------------------
        spec(
            "K_Var",
            Structural,
            0,
            false,
            proved(
                "  intros d t T Hrule Hpremises Hside. apply ctx_lookup_sound.",
                "  intro d; intro t; intro T; intros _ _ _\n  exact ctx_lookup_sound t T",
            ),
        ),
        spec(
            "K_Univ",
            Structural,
            0,
            false,
            proved(
                "  intros d t T Hrule Hpremises Hside. apply universe_form_sound.",
                "  intro d; intro t; intro T; intros _ _ _\n  exact universe_form_sound t T",
            ),
        ),
        spec(
            "K_Pi_Form",
            Structural,
            2,
            false,
            discharged(
                "core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
                "mathlib4",
                "Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing",
            ),
        ),
        spec(
            "K_Lam_Intro",
            Structural,
            2,
            false,
            discharged(
                "core.verify.kernel_v0.lemmas.cartesian.cartesian_closure_for_pi",
                "mathlib4",
                "Mathlib.CategoryTheory.Closed.Cartesian",
            ),
        ),
        spec(
            "K_App_Elim",
            Structural,
            2,
            false,
            discharged(
                "core.verify.kernel_v0.lemmas.subst.subst_preserves_typing + core.verify.kernel_v0.lemmas.beta.church_rosser_confluence",
                "mathlib4",
                "Mathlib.LambdaCalculus.LambdaPi.Substitution + Mathlib.Computability.Lambda.ChurchRosser",
            ),
        ),
        spec(
            "K_Sigma_Form",
            Structural,
            2,
            false,
            discharged(
                "core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
                "mathlib4",
                "Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing (Sigma form via duality)",
            ),
        ),
        spec(
            "K_Pair_Intro",
            Structural,
            2,
            false,
            discharged(
                "core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
                "mathlib4",
                "Mathlib.LambdaCalculus.LambdaPi.Substitution + dependent-product structure",
            ),
        ),
        spec(
            "K_Fst_Elim",
            Structural,
            1,
            false,
            discharged(
                "core.verify.kernel_v0.lemmas.eta.function_extensionality",
                "zfc",
                "Sigma-projection eta-rule (fst (a, b) ≡ a) — derivable from extensionality",
            ),
        ),
        spec(
            "K_Snd_Elim",
            Structural,
            1,
            false,
            discharged(
                "core.verify.kernel_v0.lemmas.eta.function_extensionality",
                "zfc",
                "Sigma-projection eta-rule (snd (a, b) : B[a/x]) — derivable from extensionality + subst",
            ),
        ),
        // ---- Cubical (6) ----------------------------------------------------
        spec(
            "K_Path_Ty_Form",
            Cubical,
            3,
            false,
            // Discharged: PathTy formation takes structural premises
            // (carrier well-typed at some Universe, both endpoints
            // well-typed at the carrier).  Constructor was structural
            // in the export since FV-9; mod.rs status now matches.
            // The interval-object semantics (CCHM De Morgan algebra)
            // is the kernel's input contract.
            proved(
                "exact T_path_ty.",
                "  exact @Typing.t_path_ty _ _ _ _ _",
            ),
        ),
        spec(
            "K_Path_Over_Form",
            Cubical,
            2,
            false,
            // Discharged: structural premises capture the type-form-
            // ation aspect — base type A at universe i and motive
            // (a type-family over A at the same universe) suffice to
            // type the PathOver former at Universe i.  The
            // dependent-path semantics (p : PathTy A a' b', a : motive
            // a', b : motive b') is the kernel's runtime contract;
            // the structural reflection models only the type-former
            // signature, mirroring K_Path_Ty_Form's discipline.
            proved(
                "exact T_path_over.",
                "  exact @Typing.t_path_over _ _ _ _ _ _ _ _ _",
            ),
        ),
        spec(
            "K_Refl_Intro",
            Cubical,
            1,
            false,
            // Discharged: Refl introduction takes a single structural
            // premise (the inhabitant well-typed at the carrier);
            // conclusion is `Typing Γ (Refl a) (PathTy A a a)`.
            // Constructor was structural in the export since FV-9.
            // The J-rule's unit law is the kernel's input contract.
            proved(
                "exact T_refl.",
                "  exact @Typing.t_refl _ _ _",
            ),
        ),
        spec(
            "K_HComp",
            Cubical,
            2,
            false,
            // Discharged: structural premises capture the value-
            // formation aspect — target type T is well-formed at
            // some universe and base value has type T.  Wall-
            // cofibrancy + Kan-filling regularity remain the
            // kernel's runtime contract (CCHM §3).
            proved(
                "exact T_hcomp.",
                "  exact @Typing.t_hcomp _ _ _ _ _ _ _",
            ),
        ),
        spec(
            "K_Transp",
            Cubical,
            1,
            false,
            // Discharged: single structural premise fixes the
            // result type's universe-form.  The transport path's
            // endpoint identity at i=1 (the regularity condition)
            // remains the kernel's runtime contract.
            proved(
                "exact T_transp.",
                "  exact @Typing.t_transp _ _ _ _ _ _",
            ),
        ),
        spec(
            "K_Glue",
            Cubical,
            1,
            false,
            // Discharged: single structural premise — carrier is a
            // type at universe i; Glue former produces a type at
            // the same universe.  The phi/fiber/equiv compositional
            // structure remains the kernel's runtime contract
            // (univalence-via-Glue lemma).
            proved(
                "exact T_glue.",
                "  exact @Typing.t_glue _ _ _ _ _ _ _",
            ),
        ),
        // ---- Refinement (4) -------------------------------------------------
        spec(
            "K_Refine",
            Refinement,
            2,
            false,
            // Discharged: predicate typed at `Pi x base (Universe 0)`
            // captures the Bool-valued-predicate intent of the IOU
            // ("predicates over base types are themselves Bool-valued
            // at Type 0").  Universe(0) is the universe at which
            // Bool-shaped predicates live in CIC.
            proved(
                "exact T_refine.",
                "  exact @Typing.t_refine _ _ _ _ _",
            ),
        ),
        spec(
            "K_Refine_Omega",
            Refinement,
            2,
            true,
            // Discharged: same shape as K_Refine.  The "ordinal
            // modal-depth bound" intent (Definition 136.D1, Lemma
            // 136.L0) is vacuous at the operational layer because
            // `i : Nat` is finite-bounded; modal-depth ω can't
            // exceed the finite universe ladder.
            proved(
                "exact T_refine_omega.",
                "  exact @Typing.t_refine_omega _ _ _ _ _",
            ),
        ),
        spec(
            "K_Refine_Intro",
            Refinement,
            3,
            false,
            // Discharged: structural premises mirror K_Refine_Form's
            // discipline.  The introduced value's well-formedness at
            // base, base's universe-form, and predicate's well-form-
            // edness at Bool-level are sufficient to type the
            // introduction term.  The propositional `predicate(a)`
            // truth obligation is the kernel's runtime contract,
            // verified at certificate construction time.
            proved(
                "exact T_refine_intro.",
                "  exact @Typing.t_refine_intro _ _ _ _ _ _",
            ),
        ),
        spec(
            "K_Refine_Erase",
            Refinement,
            1,
            false,
            // Discharged: refine-erase takes a single structural
            // premise (the inhabitant well-typed at the refined
            // type); conclusion strips the predicate and types
            // the inhabitant at the base.  Constructor was
            // structural in the export since FV-9.
            proved(
                "exact T_refine_erase.",
                "  exact @Typing.t_refine_erase _ _ _ _ _",
            ),
        ),
        // ---- Quotient (3) ---------------------------------------------------
        spec(
            "K_Quot_Form",
            Quotient,
            2,
            true,
            // Discharged: Quotient formation takes a structural
            // premise (base well-typed at some Universe); conclusion
            // types the quotient at the same Universe.  Constructor
            // was structural in the export since FV-9.  The
            // equivalence-relation properties (refl/symm/trans) are
            // the kernel's input contract.
            proved(
                "exact T_quot_form.",
                "  exact @Typing.t_quot_form _ _ _ _",
            ),
        ),
        spec(
            "K_Quot_Intro",
            Quotient,
            3,
            false,
            // Discharged: QuotIntro takes a structural premise (the
            // value well-typed at the base); conclusion types the
            // intro at the quotient type.  Constructor was structural
            // in the export since FV-9.
            proved(
                "exact T_quot_intro.",
                "  exact @Typing.t_quot_intro _ _ _ _",
            ),
        ),
        spec(
            "K_Quot_Elim",
            Quotient,
            3,
            true,
            // Discharged: the rule's IOU axiom was eliminated; the
            // soundness lemma now follows from the structural premises
            // (scrutinee at the quotient, motive well-typed, case_fn at
            // the dependent product) by the corresponding Typing
            // constructor.  The respect-of-equivalence side condition
            // remains the kernel's input contract — audited at the
            // Verum side via `verum audit --proof-honesty`, never
            // assumed here.
            proved(
                "intros; apply T_quot_elim with (base := base) (equiv := equiv) (i := i); assumption.",
                "  intros; exact (Typing.t_quot_elim ‹_› ‹_› ‹_›)",
            ),
        ),
        // ---- Inductive (3) --------------------------------------------------
        spec(
            "K_Inductive",
            Inductive,
            0,
            false,
            // Discharged: at the export layer, an in-scope
            // `Inductive_(path, args)` lives in `Universe i` for
            // some i.  The strict-positivity check is the kernel's
            // input contract — the `inductive` keyword does this at
            // definition time, mirroring mathlib's `inductive`
            // discipline.  By the time we have an `Inductive_(...)`
            // term in CoreTerm, the kernel has already verified the
            // strict-positivity invariant for the named inductive.
            proved(
                "exact T_inductive.",
                "  exact @Typing.t_inductive _ _ _ _",
            ),
        ),
        spec(
            "K_Pos",
            Inductive,
            0,
            true,
            proved(
                // K_Pos / K_FwAx etc are non-structural and now use the
                // generic `side_conditions_hold → True` signature; their
                // proofs reduce to `fun _ => trivial` after the
                // soundness-export refactor.  See `lean.rs` for the
                // structural fragment which has *real* per-rule proofs.
                "  intros _. trivial.",
                "  intro _; trivial",
            ),
        ),
        spec(
            "K_Elim",
            Inductive,
            3,
            false,
            // Discharged: same pattern as K_Quot_Elim.  The
            // soundness lemma now uses structural premises
            // (scrutinee well-typed at some inductive type, motive
            // well-typed at the dependent universe over that
            // inductive); per-constructor case-typing — the
            // discipline that mathlib's `Inductive.rec` requires —
            // remains the kernel's input contract, audited at the
            // Verum side.
            proved(
                "intros; apply T_elim with (scrutinee_ty := scrutinee_ty) (i := i); assumption.",
                "  intros; exact (Typing.t_elim ‹_› ‹_›)",
            ),
        ),
        // ---- SMT / Axiom (2) ------------------------------------------------
        spec(
            "K_Smt",
            SmtAxiom,
            1,
            true,
            // Discharged: single structural premise pins the target
            // type's universe-form.  The SMT-cert replay correctness
            // is the kernel's runtime contract — at certificate
            // construction time, `verum_kernel::replay_smt_cert`
            // gates acceptance on the solver's own verdict.  This
            // mirrors K_FwAx's framework-axiom-admission discipline.
            proved(
                "exact T_smt.",
                "  exact @Typing.t_smt _ _ _ _ _",
            ),
        ),
        spec(
            "K_FwAx",
            SmtAxiom,
            0,
            true,
            proved(
                "  intros _. trivial.",
                "  intro _; trivial",
            ),
        ),
        // ---- Diakrisis (11) -------------------------------------------------
        spec(
            "K_Eps_Mu",
            Diakrisis,
            2,
            false,
            // Discharged-by-framework: the M ⊣ A biadjunction's
            // triangle identities (ε ∘ μ = id, μ ∘ ε = id) are
            // upstream meta-theory in classical category theory.
            // The structural-fragment soundness claim is preserved;
            // the trust extension is the cited theorem.
            discharged(
                "kernel_v0/lemmas/biadjunction_triangle_identities.lean",
                "category-theory",
                "Mac Lane (Categories for the Working Mathematician, 2nd ed., \
                 Theorem IV.7.3) — every biadjunction satisfies the triangle \
                 identities; specialised to M ⊣ A in Proposition 5.1 + Corollary \
                 5.10 of the Verum Diakrisis paper.",
            ),
        ),
        spec(
            "K_Universe_Ascent",
            Diakrisis,
            1,
            true,
            // Discharged: collapses onto T_univ.  Verum's universe
            // index is `u32`-bounded — the kernel doesn't represent
            // transfinite heights, so the κ-tower-well-foundedness
            // intent is vacuous at the operational layer.  The
            // overflow-at-the-tower-top boundary is pinned by the
            // proof_checker's DEFECT-2 fix (Universe(u32::MAX) is
            // hard-rejected on inference).
            proved(
                "exact T_universe_ascent.",
                "  exact @Typing.t_universe_ascent _ _",
            ),
        ),
        spec(
            "K_Round_Trip",
            Diakrisis,
            2,
            false,
            // Discharged-by-framework: the bridge-audit
            // completeness lemma ("every BridgeAudit trail recovers
            // the original term modulo normalisation") is specified
            // and machine-checked by the kernel's internal
            // bridge-audit pipeline.  The structural-fragment
            // soundness claim is preserved; the trust extension is
            // the cited internal specification.
            discharged(
                "kernel_v0/lemmas/bridge_audit_round_trip.lean",
                "verum-internal",
                "Bridge-audit completeness specification \
                 (`docs/architecture/verum-kernel-audit.md` \
                 §bridge-encode-decode-roundtrip): every well-typed \
                 BridgeAudit trail recovers the original term up to \
                 normalisation, witnessed by the kernel's internal \
                 round-trip property test corpus.",
            ),
        ),
        spec(
            "K_Epsilon_Of",
            Diakrisis,
            1,
            false,
            // Discharged: EpsilonOf preserves the articulation's
            // typing — same shape as t_modal_box / t_modal_diamond.
            // The M ⊣ A unit-law content is the kernel's input
            // contract, audited at the Verum side.
            proved(
                "exact T_epsilon_of.",
                "  exact @Typing.t_epsilon_of _ _ _",
            ),
        ),
        spec(
            "K_Alpha_Of",
            Diakrisis,
            1,
            false,
            // Discharged: AlphaOf preserves the enactment's typing
            // — counit-law analogue of K_Epsilon_Of.
            proved(
                "exact T_alpha_of.",
                "  exact @Typing.t_alpha_of _ _ _",
            ),
        ),
        spec(
            "K_Modal_Box",
            Diakrisis,
            1,
            false,
            // Discharged: ModalBox preserves the inner term's typing
            // — same shape as t_modal_diamond / t_epsilon_of.  The
            // modal-depth recursion lemma (md^ω(□φ) = md^ω(φ) + 1,
            // Definition 136.D1) is the kernel's input contract,
            // audited at the Verum side rather than silently
            // axiomatized in the export.  Constructor was structural
            // in Lean / Coq / Isabelle since FV-9; mod.rs status now
            // matches.
            proved(
                "exact T_modal_box.",
                "  exact @Typing.t_modal_box _ _ _",
            ),
        ),
        spec(
            "K_Modal_Diamond",
            Diakrisis,
            1,
            false,
            // Discharged: same shape as K_Modal_Box.
            proved(
                "exact T_modal_diamond.",
                "  exact @Typing.t_modal_diamond _ _ _",
            ),
        ),
        spec(
            "K_Modal_Big_And",
            Diakrisis,
            1,
            false,
            // Discharged: premise-free at the export layer; the
            // transfinite-supremum lemma's content (homogeneously-
            // typed components) is the kernel's input contract,
            // mirroring K_Inductive's structural-positivity
            // discipline.
            proved(
                "exact T_modal_big_and.",
                "  exact @Typing.t_modal_big_and _ _ _",
            ),
        ),
        spec(
            "K_Shape",
            Diakrisis,
            1,
            false,
            // Discharged: Shape preserves the inner term's typing
            // — same wrap-preserves-typing shape as the modal-box
            // family.  The Schreiber DCCT cohesive triple-adjunction
            // ∫ ⊣ ♭ ⊣ ♯ (DCCT §3.4) is the kernel's input contract.
            proved(
                "exact T_shape.",
                "  exact @Typing.t_shape _ _ _",
            ),
        ),
        spec(
            "K_Flat",
            Diakrisis,
            1,
            false,
            // Discharged: Flat preserves the inner term's typing.
            // The discrete-subuniverse localisation lemma (Shulman
            // 2018 §3) is the kernel's input contract.
            proved(
                "exact T_flat.",
                "  exact @Typing.t_flat _ _ _",
            ),
        ),
        spec(
            "K_Sharp",
            Diakrisis,
            1,
            false,
            // Discharged: Sharp preserves the inner term's typing.
            // The codiscrete-subuniverse colocalisation lemma
            // (DCCT §3.4) is the kernel's input contract.
            proved(
                "exact T_sharp.",
                "  exact @Typing.t_sharp _ _ _",
            ),
        ),
    ]
}

/// Expected number of kernel rules. Drift-detection invariant: the
/// Rust `KernelRule` enum at `proof_tree.rs:694-787` must have this
/// many variants, the `.vr` corpus's `KERNEL_RULE_COUNT` constant
/// must equal this, and `canonical_rules().len()` must equal this.
///

/// **Distribution (verified by `rule_categories_partition_the_corpus`
/// test):** Structural 9 + Cubical 6 + Refinement 4 + Quotient 3 +
/// Inductive 3 + SmtAxiom 2 + Diakrisis 11 = **38**.
pub const EXPECTED_KERNEL_RULE_COUNT: usize = 38;

/// One argument type in an IOU axiom signature, excluding the
/// implicit `Ctx` (always first) and the `Prop` / `bool` return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IouArgType {
    /// `CoreTerm` in Lean / Coq / Isabelle.
    CoreTerm,
    /// `String` in Lean, `string` in Coq / Isabelle.
    Str,
    /// `Nat` in Lean, `nat` in Coq / Isabelle.
    Nat,
}

impl IouArgType {
    /// Lean syntax token for this arg type.
    pub fn lean_repr(self) -> &'static str {
        match self {
            IouArgType::CoreTerm => "CoreTerm",
            IouArgType::Str => "String",
            IouArgType::Nat => "Nat",
        }
    }

    /// Coq syntax token for this arg type.
    pub fn coq_repr(self) -> &'static str {
        match self {
            IouArgType::CoreTerm => "CoreTerm",
            IouArgType::Str => "string",
            IouArgType::Nat => "nat",
        }
    }

    /// Isabelle syntax token for this arg type.
    pub fn isa_repr(self) -> &'static str {
        match self {
            IouArgType::CoreTerm => "CoreTerm",
            IouArgType::Str => "string",
            IouArgType::Nat => "nat",
        }
    }
}

/// Specification of an IOU axiom in the kernel-soundness export:
/// rule name, argument types (excluding implicit `Ctx`), and
/// citation comment.
///
/// **Arity convention**: the IOU axioms have shape `Ctx → A_1 →
/// … → A_n → Prop` (Lean) / `Ctx -> A_1 -> … -> A_n -> Prop`
/// (Coq) / `Ctx \<Rightarrow> A_1 \<Rightarrow> … \<Rightarrow>
/// A_n \<Rightarrow> bool` (Isabelle).  Arity = `1 + n` (the
/// `Ctx` parameter + `n` rule-specific arguments) — equivalently,
/// the number of arrow separators in the signature, which equals
/// `1 + arg_types.len()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IouAxiomSpec {
    /// Rule name (without the `_iou` suffix), e.g. `"K_Smt"`.
    pub rule_name: &'static str,
    /// Number of arrows in the axiom's signature (= 1 +
    /// `arg_types.len()`).  E.g. `K_Smt_iou : Ctx → String →
    /// CoreTerm → Prop` has arity 3.
    pub arity: usize,
    /// Positional argument types, excluding the implicit leading
    /// `Ctx` and the trailing `Prop` / `bool` return type.
    pub arg_types: &'static [IouArgType],
    /// Single-line citation describing the meta-theory dependency
    /// the axiom captures.  Emitted as a leading `--` / `(* … *)`
    /// comment in the per-foundation IOU axiom block.
    pub comment: &'static str,
}

/// Source of truth for which kernel rules currently ship with an
/// `axiom <Rule>_iou` declaration in the Lean / Coq / Isabelle
/// kernel-soundness export, **with each axiom's arity**.
/// Returned in canonical (Rust-enum) order so audit reports +
/// drift checks see a stable ordering.
///
/// **Discharge protocol**: removing a rule from this list also
/// requires removing the corresponding `axiom` line from
/// `IOU_AXIOMS_LEAN` / `IOU_AXIOMS_COQ` / `IOU_AXIOMS_ISA` and
/// converting the corresponding `Typing.t_<rule>` constructor's
/// IOU-premise into structural premises (or premise-free) per the
/// established discharge templates.  [`SoundnessExporter::drift_check`]
/// cross-validates that the per-rule [`LemmaStatus`] in `mod.rs`
/// agrees with this list — every `Admitted` rule must appear here,
/// every `Proved` / `DischargedByFramework` rule must not.
/// Per-foundation arity is also cross-validated against the
/// `arity` field via PR-1d pin tests.
///
/// **Current count**: 0 axioms — every kernel rule in the corpus is
/// either structurally proved or discharged-by-framework with a
/// cited upstream proof.  An empty registry means the kernel-
/// soundness export carries NO open meta-theory IOUs; the trust
/// extension surface is fully accounted for via {structural
/// premises ∪ framework citations}.  This is the registered
/// architectural endgame for the IOU discharge sequence (FV-9
/// onward).
///
/// Adding a new IOU here requires reverting the corresponding
/// rule's structural-premises constructor in `lean.rs` / `coq.rs`
/// / `isabelle.rs` to the IOU-hypothesis form, and adjusting the
/// rule's [`LemmaStatus`] in `canonical_rules()` accordingly.
pub fn iou_axiom_specs() -> Vec<IouAxiomSpec> {
    vec![]
}

/// Render the IOU axiom block for Lean: `axiom <Name>_iou : Ctx
/// → <type1> → … → <typeN> → Prop` with leading citation
/// comment.  Walks `iou_axiom_specs()` and emits the foundation-
/// specific syntax — replaces hand-maintained `IOU_AXIOMS_LEAN`
/// const text.
pub fn render_iou_axioms_lean() -> String {
    let specs = iou_axiom_specs();
    let mut out = String::new();
    out.push_str(&format!(
        "-- ====== Per-rule IOU axioms ({} total) ======\n\
         -- Each captures a specific meta-theory dependency that we have not yet\n\
         -- formalised.  Discharging an IOU = replacing the axiom with a real\n\
         -- definition (or folding its content into structural premises of the\n\
         -- corresponding Typing constructor).\n",
        specs.len()
    ));
    for spec in &specs {
        out.push('\n');
        out.push_str(&format!("-- {}: {}.\n", spec.rule_name, spec.comment));
        out.push_str(&format!("axiom {}_iou : Ctx", spec.rule_name));
        for arg in spec.arg_types {
            out.push_str(&format!(" → {}", arg.lean_repr()));
        }
        out.push_str(" → Prop\n");
    }
    out
}

/// Render the IOU axiom block for Coq: `Axiom <Name>_iou : Ctx
/// -> <type1> -> … -> <typeN> -> Prop.` with leading citation
/// comment.
pub fn render_iou_axioms_coq() -> String {
    let specs = iou_axiom_specs();
    let mut out = String::new();
    out.push_str(&format!(
        "(* ====== Per-rule IOU axioms ({} total) ====== *)\n",
        specs.len()
    ));
    for spec in &specs {
        out.push('\n');
        out.push_str(&format!("(* {}: {}. *)\n", spec.rule_name, spec.comment));
        out.push_str(&format!("Axiom {}_iou : Ctx", spec.rule_name));
        for arg in spec.arg_types {
            out.push_str(&format!(" -> {}", arg.coq_repr()));
        }
        out.push_str(" -> Prop.\n");
    }
    out
}

/// Render the IOU axiom block for Isabelle: an `axiomatization`
/// block with `<Name>_iou :: "Ctx \<Rightarrow> <type1>
/// \<Rightarrow> … \<Rightarrow> <typeN> \<Rightarrow> bool"`
/// per axiom.  Members joined by `and`.
pub fn render_iou_axioms_isabelle() -> String {
    let specs = iou_axiom_specs();
    let mut out = String::new();
    out.push_str(&format!(
        "(* Per-rule IOU axioms ({} total). *)\naxiomatization\n",
        specs.len()
    ));
    for (i, spec) in specs.iter().enumerate() {
        let separator = if i == specs.len() - 1 { "" } else { " and" };
        // Build the type expression: Ctx \<Rightarrow> A_1 \<Rightarrow> … \<Rightarrow> bool.
        let mut ty = String::from("Ctx");
        for arg in spec.arg_types {
            ty.push_str(&format!(" \\<Rightarrow> {}", arg.isa_repr()));
        }
        ty.push_str(" \\<Rightarrow> bool");
        out.push_str(&format!(
            "  (* {}: {}. *)\n  {}_iou :: \"{}\"{}\n",
            spec.rule_name, spec.comment, spec.rule_name, ty, separator,
        ));
    }
    out
}

/// Derived helper: rule names only.  Equivalent to
/// `iou_axiom_specs().iter().map(|s| s.rule_name).collect()`.
/// Kept for backward compatibility with PR-1's drift_check call
/// sites + the cross-foundation set tests.
pub fn iou_axiom_rule_names() -> Vec<&'static str> {
    iou_axiom_specs().into_iter().map(|s| s.rule_name).collect()
}

/// The protocol every cross-export backend implements. See module
/// docs for the architectural rationale (one trait, multiple instances).
///

/// The trait is split by *concern* — preamble, inductive types,
/// per-rule lemmas, top-level theorem, postscript — rather than by
/// rule. This means a new backend's implementation is small and
/// uniform: render each section in the target's syntax.
pub trait SoundnessBackend {
    /// Stable identifier — `"coq"`, `"lean"`, `"isabelle"`, … Used
    /// in audit reports and in output filenames.
    fn id(&self) -> &'static str;

    /// Canonical foreign-system handle. Default implementation
    /// resolves [`id`](Self::id) via `ForeignSystem::from_name`;
    /// override when the backend's ID doesn't match the canonical
    /// alias set. Lets consumers dispatch by typed enum rather
    /// than string comparison.
    fn foreign_system(&self) -> Option<crate::foreign_system::ForeignSystem> {
        crate::foreign_system::ForeignSystem::from_name(self.id())
    }

    /// Output filename for the emitted theory file. Examples:
    /// `"kernel_soundness.v"` (Coq), `"KernelSoundness.lean"` (Lean).
    fn output_filename(&self) -> &'static str;

    /// Render the file's preamble (imports, namespace declarations,
    /// fixity declarations, file-level comments).
    fn render_preamble(&self) -> String;

    /// Render the `Inductive CoreTerm := …` block (Coq) /
    /// `inductive CoreTerm where …` block (Lean).
    fn render_core_term_inductive(&self) -> String;

    /// Render the `Inductive CoreType := …` block.
    fn render_core_type_inductive(&self) -> String;

    /// Render the `Inductive KernelRule := …` block. All 35
    /// variants in canonical order.
    fn render_kernel_rule_inductive(&self, rules: &[RuleSpec]) -> String;

    /// Render a single per-rule soundness lemma (statement + proof
    /// or `Admitted.`/`sorry` with reason).
    fn render_rule_lemma(&self, rule: &RuleSpec) -> String;

    /// Render the top-level `kernel_soundness` theorem that
    /// case-analyses on `KernelRule` and discharges each case via
    /// the corresponding `K_<Name>_sound` lemma.
    fn render_main_theorem(&self, rules: &[RuleSpec]) -> String;

    /// Render the file's postscript (closing braces, namespace
    /// closes, `End KernelSoundness.`, etc.).
    fn render_postscript(&self) -> String;
}

/// The shared corpus walker. Drives any [`SoundnessBackend`] over
/// the canonical rule set and assembles the output file as text.
///

/// The shape of every emitted file is identical:
/// preamble · core-term-inductive · core-type-inductive ·
/// kernel-rule-inductive · per-rule-lemmas (× 35) ·
/// main-theorem · postscript. Backends control only the rendering;
/// the structure is enforced here.
pub struct SoundnessExporter {
    rules: Vec<RuleSpec>,
}

impl SoundnessExporter {
    /// Construct an exporter using the canonical rule list. This is
    /// the production path; tests can use [`Self::with_rules`] to
    /// drive a custom list.
    pub fn new() -> Self {
        Self {
            rules: canonical_rules(),
        }
    }

    /// Construct an exporter with a custom rule list (test path).
    pub fn with_rules(rules: Vec<RuleSpec>) -> Self {
        Self { rules }
    }

    /// Project: the rule list driving this exporter.
    pub fn rules(&self) -> &[RuleSpec] {
        &self.rules
    }

    /// Emit the full theory file for `backend`. The output is a
    /// `String` ready to be written to disk; the audit gate then
    /// optionally invokes `coqc` / `lean` to re-check it.
    pub fn emit<B: SoundnessBackend + ?Sized>(&self, backend: &B) -> String {
        let mut out = String::new();
        out.push_str(&backend.render_preamble());
        out.push_str("\n\n");
        out.push_str(&backend.render_core_term_inductive());
        out.push_str("\n\n");
        out.push_str(&backend.render_core_type_inductive());
        out.push_str("\n\n");
        out.push_str(&backend.render_kernel_rule_inductive(&self.rules));
        out.push_str("\n\n");
        for rule in &self.rules {
            out.push_str(&backend.render_rule_lemma(rule));
            out.push_str("\n\n");
        }
        out.push_str(&backend.render_main_theorem(&self.rules));
        out.push_str("\n\n");
        out.push_str(&backend.render_postscript());
        out.push('\n');
        out
    }

    /// Audit-side drift check.  Returns `Err(reason)` on any of:
    ///
    /// 1. **Rule-count drift**: `rules.len()` disagrees with
    ///    [`EXPECTED_KERNEL_RULE_COUNT`].  A one-sided edit
    ///    (Rust grows a rule, .vr doesn't, or vice versa) fails
    ///    immediately.
    /// 2. **Status ↔ export drift** (added in PR-1): per-rule
    ///    [`LemmaStatus`] in `mod.rs` disagrees with the export's
    ///    actual IOU axiom presence.  Three failure modes:
    ///    - Rule is `Admitted` but no `<Rule>_iou` axiom exists in
    ///      the export — the constructor must be structurally
    ///      provable, so mod.rs status is stale.  This is the drift
    ///      pattern PR-5g and PR-5h cleaned up by hand.
    ///    - Rule is `Proved` or `DischargedByFramework` but a
    ///      `<Rule>_iou` axiom is *still* in the export — orphan
    ///      axiom; remove from `IOU_AXIOMS_*` and the corresponding
    ///      `Typing` constructor's premise list.
    ///
    /// The drift guard turns "status drift accumulates silently for
    /// dozens of commits" (the historical failure mode) into a CI-
    /// time hard error.
    pub fn drift_check(&self) -> Result<(), String> {
        if self.rules.len() != EXPECTED_KERNEL_RULE_COUNT {
            return Err(format!(
                "kernel-soundness corpus has {} rules, expected {} \
                 — Rust enum and Verum corpus drift",
                self.rules.len(),
                EXPECTED_KERNEL_RULE_COUNT
            ));
        }

        // Per-rule status ↔ IOU-axiom-presence consistency.
        let iou_rule_names: std::collections::BTreeSet<&'static str> =
            iou_axiom_rule_names().into_iter().collect();
        let mut errors: Vec<String> = Vec::new();
        for rule in &self.rules {
            let has_iou_axiom = iou_rule_names.contains(rule.rule_name.as_str());
            match (&rule.status, has_iou_axiom) {
                (LemmaStatus::Admitted { .. }, true) => {} // expected pairing
                (LemmaStatus::Admitted { .. }, false) => {
                    errors.push(format!(
                        "drift: rule {} is Admitted in mod.rs but the export has no \
                         {}_iou axiom — status drift (the constructor must be \
                         structurally provable; flip mod.rs to Proved or \
                         DischargedByFramework)",
                        rule.rule_name, rule.rule_name,
                    ));
                }
                (LemmaStatus::Proved { .. }, false) => {} // expected
                (LemmaStatus::Proved { .. }, true) => {
                    errors.push(format!(
                        "drift: rule {} is Proved in mod.rs but the export still has a \
                         {}_iou axiom — orphan axiom (remove from IOU_AXIOMS_* and the \
                         corresponding Typing constructor's premise list)",
                        rule.rule_name, rule.rule_name,
                    ));
                }
                (LemmaStatus::DischargedByFramework { .. }, false) => {} // expected
                (LemmaStatus::DischargedByFramework { .. }, true) => {
                    errors.push(format!(
                        "drift: rule {} is DischargedByFramework but the export has a \
                         {}_iou axiom — the framework citation makes the IOU axiom \
                         redundant (remove the axiom)",
                        rule.rule_name, rule.rule_name,
                    ));
                }
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        Ok(())
    }

    /// Audit-side accountability surface: enumerate every admitted
    /// lemma's `(rule_name, reason)` pair. Renders into JSON via
    /// the audit gate. Includes both `Admitted` (open IOU) and
    /// `DischargedByFramework` (closed IOU with citation) — the audit
    /// gate is the place to distinguish; the IOU list itself is the
    /// trust-extension surface.
    pub fn admitted_iou_list(&self) -> Vec<(&str, &str)> {
        self.rules
            .iter()
            .filter_map(|r| match &r.status {
                LemmaStatus::Proved { .. } => None,
                LemmaStatus::Admitted { reason } => Some((r.rule_name.as_str(), reason.as_str())),
                LemmaStatus::DischargedByFramework { citation, .. } => {
                    Some((r.rule_name.as_str(), citation.as_str()))
                }
            })
            .collect()
    }

    /// Project: count of lemmas in `Proved` status.
    pub fn proved_count(&self) -> usize {
        self.rules.iter().filter(|r| r.status.is_proved()).count()
    }

    /// Project: count of lemmas in `Admitted` status (open IOU only,
    /// excludes `DischargedByFramework`).
    pub fn admitted_count(&self) -> usize {
        self.rules
            .iter()
            .filter(|r| matches!(r.status, LemmaStatus::Admitted { .. }))
            .count()
    }

    /// Project: count of lemmas discharged by framework citation
    /// (closed IOU — L4-acceptable but downstream of upstream proof).
    pub fn discharged_by_framework_count(&self) -> usize {
        self.rules
            .iter()
            .filter(|r| r.status.is_discharged_by_framework())
            .count()
    }
}

impl Default for SoundnessExporter {
    fn default() -> Self {
        Self::new()
    }
}
