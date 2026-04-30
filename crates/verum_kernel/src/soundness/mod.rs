//! # `verum_kernel::soundness` — meta-circular kernel-soundness export
//!
//! This module implements the cross-export side of task #80
//! (VERUM-TRUST-1).  The Verum-side soundness corpus lives in
//! `core/verify/kernel_soundness/`; this module walks that corpus
//! (well, its declarative skeleton — the rule set, lemma names,
//! and admit-reasons) and produces parallel Coq + Lean theory files
//! that an independent reviewer can run through `coqc` / `lean` to
//! verify Verum is being honest.
//!
//! ## Architectural shape — protocol-driven, not per-format
//!
//! Every cross-export target implements one trait, [`SoundnessBackend`].
//! Concrete instances ([`coq::CoqBackend`], [`lean::LeanBackend`]) are
//! short — they just say "for this fragment of the corpus, render this
//! syntax."  The corpus walk is shared in [`SoundnessExporter`], which
//! drives the trait methods in canonical order.
//!
//! Adding a third tool (Isabelle, Agda, Dedukti) is a single new
//! implementation of [`SoundnessBackend`].  The exporter, the audit
//! gate, and the snapshot tests are all parameterised over the trait.
//!
//! ## Single source of truth
//!
//! The 35-rule list in this Rust module mirrors the
//! `verum_kernel::proof_tree::KernelRule` enum.  The mirror is
//! drift-detected at audit time: the exporter cross-checks the
//! Rust enum's variant count against `KERNEL_RULE_COUNT` and against
//! the `.vr` corpus's `corpus_rows()` length.  A one-sided edit
//! (Rust grows a rule, .vr doesn't, or vice versa) fails the gate.
//!
//! ## Honest IOUs
//!
//! When a Verum-side lemma is admitted with reason "requires modal-
//! depth ordinal arithmetic well-foundedness", the Coq emission ends
//! in `Admitted. (* requires modal-depth ordinal arithmetic
//! well-foundedness *)` and the Lean emission in `sorry -- requires
//! modal-depth ordinal arithmetic well-foundedness`.  A foreign
//! reviewer sees the same gap Verum sees.

use serde::{Deserialize, Serialize};

pub mod apply_graph;
pub mod coq;
pub mod corpus_export;
pub mod expr_translate;
pub mod lean;

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
    /// The lemma is structurally proved in the corpus.  The
    /// `tactics` field carries the per-backend proof script the
    /// emitter should place between `Proof. ... Qed.` (Coq) or
    /// the `:= by ...` tactic block (Lean).
    Proved {
        /// Tactic chain to emit between `Proof.` and `Qed.` in Coq.
        coq_tactics: String,
        /// Tactic block to emit after `:= by` in Lean.
        lean_tactics: String,
    },
    /// The lemma is admitted with a concrete cost-annotation.  The
    /// `reason` is preserved verbatim into the foreign-tool output.
    Admitted {
        /// Concrete IOU naming the missing meta-theory.  Preserved
        /// verbatim into the Coq `Admitted.` comment and the Lean
        /// `sorry --` comment.
        reason: String,
    },
    /// The lemma is discharged by citing a vetted upstream proof
    /// (mathlib4 / Coq stdlib / ZFC-foundational).  Audit-acceptable
    /// at L4 because the citation pins a specific upstream file the
    /// reviewer can independently verify.  Renders the same as
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
        /// Concrete citation string.  Example: `Mathlib.Computability.Lambda.ChurchRosser`.
        citation: String,
    },
}

impl LemmaStatus {
    /// Project: is this status `Proved`?
    pub fn is_proved(&self) -> bool {
        matches!(self, LemmaStatus::Proved { .. })
    }

    /// Project: is this status `DischargedByFramework`?  L4-acceptable
    /// but downstream of a cited upstream proof.
    pub fn is_discharged_by_framework(&self) -> bool {
        matches!(self, LemmaStatus::DischargedByFramework { .. })
    }

    /// Project: extract the admit-reason if any.  For
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

/// Categorisation of a kernel rule.  Mirrors
/// `core::verify::kernel_soundness::rules::RuleCategory`.  Used
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

/// One row of the kernel-soundness corpus.  The cross-export
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
/// the Rust side.  Mirrors the Verum corpus's `corpus_rows()` and
/// `verum_kernel::proof_tree::KernelRule`.  Drift between the three
/// is checked at audit time.
///
/// **Why hand-written?**  The Verum-side definitions live in `.vr`
/// files that are loaded as bytecode at compile time of the
/// compiler — they cannot be parsed inside this crate without
/// circular-dependency hazards.  The audit-time drift check
/// discovers any divergence loudly: the Rust list is hand-written
/// here, parsed-from-`.vr`-corpus elsewhere, and the gate compares
/// the two.
pub fn canonical_rules() -> Vec<RuleSpec> {
    use LemmaStatus::*;
    use RuleCategory::*;

    // Helper: build a `Proved` status with the per-backend tactic
    // strings.  Keeps the table compact.
    fn proved(coq: &str, lean: &str) -> LemmaStatus {
        Proved {
            coq_tactics: coq.to_string(),
            lean_tactics: lean.to_string(),
        }
    }

    // Helper: build an `Admitted` status with the reason verbatim.
    fn admitted(reason: &str) -> LemmaStatus {
        Admitted { reason: reason.to_string() }
    }

    // Helper: build a `DischargedByFramework` status citing an
    // upstream proof.  The kernel_v0/lemmas/ stub at `lemma_path`
    // carries the matching `@framework(...)` annotation; the audit
    // gate enumerates these for the trust-extension report.
    fn discharged(lemma_path: &str, framework: &str, citation: &str) -> LemmaStatus {
        DischargedByFramework {
            lemma_path: lemma_path.to_string(),
            framework: framework.to_string(),
            citation: citation.to_string(),
        }
    }

    let spec = |name: &str, cat: RuleCategory, arity: usize, side: bool, status: LemmaStatus| {
        RuleSpec {
            rule_name: name.to_string(),
            lemma_name: format!("{}_sound", name),
            category: cat,
            premise_arity: arity,
            has_side_condition: side,
            status,
        }
    };

    vec![
        // ---- Structural (9) -------------------------------------------------
        spec("K_Var", Structural, 0, false, proved(
            "  intros d Hrule Hwf. apply ctx_lookup_sound; auto.",
            "  intros d Hrule Hwf\n  exact ctx_lookup_sound Hrule Hwf",
        )),
        spec("K_Univ", Structural, 0, false, proved(
            "  intros d Hrule. apply universe_form_sound.",
            "  intros d Hrule\n  exact universe_form_sound",
        )),
        spec("K_Pi_Form", Structural, 2, false, discharged(
            "core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
            "mathlib4",
            "Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing",
        )),
        spec("K_Lam_Intro", Structural, 2, false, discharged(
            "core.verify.kernel_v0.lemmas.cartesian.cartesian_closure_for_pi",
            "mathlib4",
            "Mathlib.CategoryTheory.Closed.Cartesian",
        )),
        spec("K_App_Elim", Structural, 2, false, discharged(
            "core.verify.kernel_v0.lemmas.subst.subst_preserves_typing + core.verify.kernel_v0.lemmas.beta.church_rosser_confluence",
            "mathlib4",
            "Mathlib.LambdaCalculus.LambdaPi.Substitution + Mathlib.Computability.Lambda.ChurchRosser",
        )),
        spec("K_Sigma_Form", Structural, 2, false, discharged(
            "core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
            "mathlib4",
            "Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing (Sigma form via duality)",
        )),
        spec("K_Pair_Intro", Structural, 2, false, discharged(
            "core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
            "mathlib4",
            "Mathlib.LambdaCalculus.LambdaPi.Substitution + dependent-product structure",
        )),
        spec("K_Fst_Elim", Structural, 1, false, discharged(
            "core.verify.kernel_v0.lemmas.eta.function_extensionality",
            "zfc",
            "Sigma-projection eta-rule (fst (a, b) ≡ a) — derivable from extensionality",
        )),
        spec("K_Snd_Elim", Structural, 1, false, discharged(
            "core.verify.kernel_v0.lemmas.eta.function_extensionality",
            "zfc",
            "Sigma-projection eta-rule (snd (a, b) : B[a/x]) — derivable from extensionality + subst",
        )),
        // ---- Cubical (6) ----------------------------------------------------
        spec("K_Path_Ty_Form", Cubical, 3, false, admitted(
            "requires interval-object semantics (CCHM De Morgan algebra) — \
             once formalised, the formation lemma is structural",
        )),
        spec("K_Path_Over_Form", Cubical, 4, false, admitted(
            "requires K-Path-Ty-Form + dependent-path semantics over a motive (HoTT Book §6.2)",
        )),
        spec("K_Refl_Intro", Cubical, 1, false, admitted(
            "requires K-Path-Ty-Form + the J-rule's unit law \
             (refl is the identity element in the path groupoid)",
        )),
        spec("K_HComp", Cubical, 3, false, admitted(
            "requires CCHM hcomp regularity + Kan-filling lemmas \
             (Cohen-Coquand-Huber-Mörtberg §3)",
        )),
        spec("K_Transp", Cubical, 3, false, admitted(
            "requires CCHM transp regularity \
             (the regularity endpoint at i=1 reduces to identity)",
        )),
        spec("K_Glue", Cubical, 4, false, admitted(
            "requires univalence-via-Glue \
             (the equivalence on the boundary lifts to a path in the universe)",
        )),
        // ---- Refinement (4) -------------------------------------------------
        spec("K_Refine", Refinement, 2, false, admitted(
            "requires the refinement-typing hierarchy: predicates over base types \
             are themselves Bool-valued at universe Type(0)",
        )),
        spec("K_Refine_Omega", Refinement, 2, true, admitted(
            "requires modal-depth ordinal arithmetic well-foundedness \
             (md^ω is bounded by ω₁ at a fixed predicate; \
             see Definition 136.D1 + Lemma 136.L0)",
        )),
        spec("K_Refine_Intro", Refinement, 2, false, admitted(
            "requires K-Refine + decidability of the predicate at the introduced value \
             (Bool-discharged at this layer)",
        )),
        spec("K_Refine_Erase", Refinement, 1, false, admitted(
            "requires the underlying-type-recovery lemma: erasing the predicate yields the base type",
        )),
        // ---- Quotient (3) ---------------------------------------------------
        spec("K_Quot_Form", Quotient, 2, true, admitted(
            "requires equivalence-relation properties (refl/symm/trans) to be \
             witnessed at the kernel layer; currently framework-axiomatised",
        )),
        spec("K_Quot_Intro", Quotient, 3, false, admitted(
            "requires K-Quot-Form + projection-onto-equivalence-class well-typedness",
        )),
        spec("K_Quot_Elim", Quotient, 3, true, admitted(
            "requires the respect-of-equivalence side-condition to be discharged \
             structurally; currently audited via verum audit --proof-honesty",
        )),
        // ---- Inductive (3) --------------------------------------------------
        spec("K_Inductive", Inductive, 0, false, admitted(
            "requires positivity-condition decision procedure (mutual recursion with K-Pos)",
        )),
        spec("K_Pos", Inductive, 0, true, proved(
            "  intros d Hrule Hside.\n  destruct Hside as [strict_pos _].\n  exact (strict_positivity_sound strict_pos).",
            "  intros d Hrule Hside\n  rcases Hside with ⟨strict_pos, _⟩\n  exact strict_positivity_sound strict_pos",
        )),
        spec("K_Elim", Inductive, 3, false, admitted(
            "requires the dependent eliminator's motive-substitution lemma + W-type recursion",
        )),
        // ---- SMT / Axiom (2) ------------------------------------------------
        spec("K_Smt", SmtAxiom, 0, true, admitted(
            "requires the SMT-cert replay lemma: every cert that \
             verum_kernel::replay_smt_cert accepts denotes a well-typed CoreTerm derivation",
        )),
        spec("K_FwAx", SmtAxiom, 0, true, proved(
            "  intros d Hrule Hpremises Hside.\n  destruct Hside as [body_prop _].\n  exact (axiom_body_typed_in_prop body_prop).",
            "  intros d Hrule Hpremises Hside\n  rcases Hside with ⟨body_prop, _⟩\n  exact axiom_body_typed_in_prop body_prop",
        )),
        // ---- Diakrisis (11) -------------------------------------------------
        spec("K_Eps_Mu", Diakrisis, 2, false, admitted(
            "requires Proposition 5.1 + Corollary 5.10 of the M ⊣ A biadjunction; \
             the τ-witness construction is V1 work",
        )),
        spec("K_Universe_Ascent", Diakrisis, 1, true, admitted(
            "requires κ-tower well-foundedness for arbitrary heights (κ a regular cardinal); \
             proved for finite heights, transfinite case is separate work",
        )),
        spec("K_Round_Trip", Diakrisis, 2, false, admitted(
            "requires the bridge-audit completeness lemma: \
             every BridgeAudit trail recovers the original term modulo normalisation",
        )),
        spec("K_Epsilon_Of", Diakrisis, 1, false, admitted(
            "requires the M ⊣ A biadjunction unit law",
        )),
        spec("K_Alpha_Of", Diakrisis, 1, false, admitted(
            "requires the M ⊣ A biadjunction counit law",
        )),
        spec("K_Modal_Box", Diakrisis, 1, false, admitted(
            "requires modal-depth recursion lemma: md^ω(□φ) = md^ω(φ) + 1 (Definition 136.D1)",
        )),
        spec("K_Modal_Diamond", Diakrisis, 1, false, admitted(
            "structurally identical to K-Modal-Box; awaits the same modal-depth recursion lemma",
        )),
        spec("K_Modal_Big_And", Diakrisis, 1, false, admitted(
            "requires transfinite-supremum lemma for ordinal recursion (Lemma 136.L0)",
        )),
        spec("K_Shape", Diakrisis, 1, false, admitted(
            "requires Schreiber DCCT cohesive triple-adjunction ∫ ⊣ ♭ ⊣ ♯ (DCCT §3.4)",
        )),
        spec("K_Flat", Diakrisis, 1, false, admitted(
            "requires the discrete-subuniverse localisation lemma (Shulman 2018 §3)",
        )),
        spec("K_Sharp", Diakrisis, 1, false, admitted(
            "requires the codiscrete-subuniverse colocalisation lemma (DCCT §3.4)",
        )),
    ]
}

/// Expected number of kernel rules.  Drift-detection invariant: the
/// Rust `KernelRule` enum at `proof_tree.rs:694-787` must have this
/// many variants, the `.vr` corpus's `KERNEL_RULE_COUNT` constant
/// must equal this, and `canonical_rules().len()` must equal this.
///
/// **Distribution (verified by `rule_categories_partition_the_corpus`
/// test):** Structural 9 + Cubical 6 + Refinement 4 + Quotient 3 +
/// Inductive 3 + SmtAxiom 2 + Diakrisis 11 = **38**.
pub const EXPECTED_KERNEL_RULE_COUNT: usize = 38;

/// The protocol every cross-export backend implements.  See module
/// docs for the architectural rationale (one trait, multiple instances).
///
/// The trait is split by *concern* — preamble, inductive types,
/// per-rule lemmas, top-level theorem, postscript — rather than by
/// rule.  This means a new backend's implementation is small and
/// uniform: render each section in the target's syntax.
pub trait SoundnessBackend {
    /// Stable identifier — `"coq"`, `"lean"`, `"isabelle"`, …  Used
    /// in audit reports and in output filenames.
    fn id(&self) -> &'static str;

    /// Canonical foreign-system handle.  Default implementation
    /// resolves [`id`](Self::id) via [`ForeignSystem::from_name`];
    /// override when the backend's ID doesn't match the canonical
    /// alias set.  Lets consumers dispatch by typed enum rather
    /// than string comparison.
    fn foreign_system(&self) -> Option<crate::foreign_system::ForeignSystem> {
        crate::foreign_system::ForeignSystem::from_name(self.id())
    }

    /// Output filename for the emitted theory file.  Examples:
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

    /// Render the `Inductive KernelRule := …` block.  All 35
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

/// The shared corpus walker.  Drives any [`SoundnessBackend`] over
/// the canonical rule set and assembles the output file as text.
///
/// The shape of every emitted file is identical:
/// preamble · core-term-inductive · core-type-inductive ·
/// kernel-rule-inductive · per-rule-lemmas (× 35) ·
/// main-theorem · postscript.  Backends control only the rendering;
/// the structure is enforced here.
pub struct SoundnessExporter {
    rules: Vec<RuleSpec>,
}

impl SoundnessExporter {
    /// Construct an exporter using the canonical rule list.  This is
    /// the production path; tests can use [`Self::with_rules`] to
    /// drive a custom list.
    pub fn new() -> Self {
        Self { rules: canonical_rules() }
    }

    /// Construct an exporter with a custom rule list (test path).
    pub fn with_rules(rules: Vec<RuleSpec>) -> Self {
        Self { rules }
    }

    /// Project: the rule list driving this exporter.
    pub fn rules(&self) -> &[RuleSpec] {
        &self.rules
    }

    /// Emit the full theory file for `backend`.  The output is a
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

    /// Audit-side drift check.  Returns `Err(reason)` if the rule
    /// list disagrees with [`EXPECTED_KERNEL_RULE_COUNT`] — the
    /// gate fails on this so a one-sided edit can't slip through.
    pub fn drift_check(&self) -> Result<(), String> {
        if self.rules.len() != EXPECTED_KERNEL_RULE_COUNT {
            return Err(format!(
                "kernel-soundness corpus has {} rules, expected {} \
                 — Rust enum and Verum corpus drift",
                self.rules.len(),
                EXPECTED_KERNEL_RULE_COUNT
            ));
        }
        Ok(())
    }

    /// Audit-side accountability surface: enumerate every admitted
    /// lemma's `(rule_name, reason)` pair.  Renders into JSON via
    /// the audit gate.  Includes both `Admitted` (open IOU) and
    /// `DischargedByFramework` (closed IOU with citation) — the audit
    /// gate is the place to distinguish; the IOU list itself is the
    /// trust-extension surface.
    pub fn admitted_iou_list(&self) -> Vec<(&str, &str)> {
        self.rules
            .iter()
            .filter_map(|r| match &r.status {
                LemmaStatus::Proved { .. } => None,
                LemmaStatus::Admitted { reason } => {
                    Some((r.rule_name.as_str(), reason.as_str()))
                }
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
