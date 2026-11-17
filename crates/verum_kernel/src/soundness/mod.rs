//! Export the kernel rule catalogue into independently inspectable theory files.
//! Lean and Coq derive closure of an abstract typing relation. Isabelle covers
//! its structural fragment. Agda records syntax and rule names. The catalogue
//! keeps native refusals, unavailable models and unverified references explicit.

use serde::{Deserialize, Serialize};
use verum_common::{List, Text};

pub mod agda;
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

/// Status of a rule in the abstract schema catalogue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LemmaStatus {    /// The executable calculus has no admission for this rule.
    Unsupported {
        /// Native refusal reason.
        reason: Text,
    },
    /// The executable judgment exists, but its authority is absent from the model.
    ModelUnavailable {
        /// Missing authority in the exported model.
        reason: Text,
    },
    /// Lean and Coq have a constructor for the stated abstract schema.
    Proved,
    /// An unresolved claim. Export records the reason but emits no theorem.
    Admitted {
        /// Missing evidence, preserved as diagnostic metadata.
        reason: String,
    },
    /// A reference associated with a still-unverified metatheory or
    /// implementation obligation. It grants no proof authority. An available
    /// abstract rule schema is independently derived from its constructors;
    /// that derivation neither checks this reference nor closes the obligation.
    UnverifiedFrameworkReference {
        /// Source associated with the unresolved obligation.
        /// Example: `core.verify.kernel_v0.lemmas.beta.church_rosser_confluence`.
        lemma_path: String,
        /// Upstream framework name (e.g. "mathlib4", "coq_stdlib", "zfc").
        framework: String,
        /// Concrete citation string. Example: `Mathlib.Computability.Lambda.ChurchRosser`.
        citation: String,
    },
}

impl LemmaStatus {
    /// Whether this rule has an exportable admission schema.
    pub fn is_supported(&self) -> bool {
        matches!(
            self,
            Self::Proved | Self::UnverifiedFrameworkReference { .. }
        )
    }

    /// Whether the row has a schema without an associated unverified reference.
    pub fn is_proved(&self) -> bool {
        matches!(self, Self::Proved)
    }

    /// Whether the row records an unverified metatheory reference.
    pub fn is_unverified_framework_reference(&self) -> bool {
        matches!(self, Self::UnverifiedFrameworkReference { .. })
    }

    /// The specific refusal, outstanding obligation, or unverified citation.
    pub fn explanation(&self) -> Option<&str> {
        match self {
            Self::Unsupported { reason } | Self::ModelUnavailable { reason } => Some(reason.as_str()),
            Self::Admitted { reason } => Some(reason.as_str()),
            Self::UnverifiedFrameworkReference { citation, .. } => Some(citation.as_str()),
            Self::Proved => None,
        }
    }

    fn omission(&self, name: &str) -> Option<String> {
        let label = match self {
            Self::Unsupported { .. } => "UNSUPPORTED",
            Self::ModelUnavailable { .. } => "MODEL UNAVAILABLE",
            Self::Admitted { .. } => "UNPROVED",
            _ => return None,
        };
        Some(format!("{label} {name}: {}", self.explanation()?))
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
    /// Catalogue premise count. A quantified family of judgments counts as one premise.
    pub premise_arity: usize,
    /// Whether the rule has a non-judgmental side-condition obligation.
    pub has_side_condition: bool,
    /// The lemma's proof status.
    pub status: LemmaStatus,
}

impl RuleSpec {
    fn export_note(&self) -> String {
        if let Some(reason) = self.status.omission(&self.rule_name) {
            return reason;
        }
        if let LemmaStatus::UnverifiedFrameworkReference {
            lemma_path,
            framework,
            citation,
        } = &self.status
        {
            return format!(
                "{} — Unverified metatheory reference. Source {lemma_path}. Framework {framework}. Citation {citation}.",
                self.rule_name
            );
        }
        format!(
            "{} — {} schema, {} premises",
            self.rule_name,
            self.category.tag(),
            self.premise_arity
        )
    }
}

/// Each proposition is used verbatim for the typing constructor, the per-rule
/// theorem and its branch of the aggregate. No alternative proposition is
/// substituted when a backend lacks a schema.
struct TypingSchema {
    constructor: &'static str,
    proposition: &'static str,
}

/// The canonical rule specification — single source of truth on
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

    fn reference(lemma_path: &str, framework: &str, citation: &str) -> LemmaStatus {
        UnverifiedFrameworkReference {
            lemma_path: lemma_path.into(),
            framework: framework.into(),
            citation: citation.into(),
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
        spec("K_Var", Structural, 0, false, Proved),
        spec("K_Univ", Structural, 0, false, Proved),
        spec("K_Pi_Form", Structural, 2, false, reference("core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
                "mathlib4",
                "Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing",
            )),
        spec("K_Lam_Intro", Structural, 2, false, reference("core.verify.kernel_v0.lemmas.cartesian.cartesian_closure_for_pi",
                "mathlib4",
                "Mathlib.CategoryTheory.Closed.Cartesian",
            )),
        spec("K_App_Elim", Structural, 2, false, reference("core.verify.kernel_v0.lemmas.subst.subst_preserves_typing + core.verify.kernel_v0.lemmas.beta.church_rosser_confluence",
                "mathlib4",
                "Mathlib.LambdaCalculus.LambdaPi.Substitution + Mathlib.Computability.Lambda.ChurchRosser",
            )),
        spec("K_Sigma_Form", Structural, 2, false, reference("core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
                "mathlib4",
                "Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing (Sigma form via duality)",
            )),
        spec("K_Pair_Intro", Structural, 3, false, reference("core.verify.kernel_v0.lemmas.subst.subst_preserves_typing",
                "mathlib4",
                "Mathlib.LambdaCalculus.LambdaPi.Substitution + dependent-product structure",
            )),
        spec("K_Fst_Elim", Structural, 1, false, reference("core.verify.kernel_v0.lemmas.eta.function_extensionality",
                "zfc",
                "Sigma-projection eta-rule (fst (a, b) ≡ a) — derivable from extensionality",
            )),
        spec("K_Snd_Elim", Structural, 1, false, reference("core.verify.kernel_v0.lemmas.eta.function_extensionality",
                "zfc",
                "Sigma-projection eta-rule (snd (a, b) : B[a/x]) — derivable from extensionality + subst",
            )),
        spec("K_Path_Ty_Form", Cubical, 3, false, Proved),
        spec("K_Path_Over_Form", Cubical, 4, false, Proved),
        spec("K_Refl_Intro", Cubical, 1, false, Proved),
        spec("K_HComp", Cubical, 2, false, Unsupported { reason: Text::from("The executable kernel refuses composition until partial-element support and boundary coherence are checked") }),
        spec("K_Transp", Cubical, 4, false, Proved),
        spec("K_Glue", Cubical, 1, false, Unsupported { reason: Text::from("The executable kernel refuses gluing until equivalence and boundary coherence are checked") }),
        spec("K_Refine", Refinement, 3, true, Proved),
        spec("K_Refine_Omega", Refinement, 3, true, ModelUnavailable { reason: Text::from("The export lacks the ordinal modal-depth inequality checked by the executable omega formation rule") }),
        spec("K_Refine_Intro", Refinement, 3, false, Proved),
        spec("K_Refine_Erase", Refinement, 1, false, Proved),
        spec("K_Quot_Form", Quotient, 3, true, Proved),
        spec("K_Quot_Intro", Quotient, 2, false, Proved),
        spec("K_Quot_Elim", Quotient, 10, true, Proved),
        spec("K_Inductive", Inductive, 0, false, ModelUnavailable { reason: Text::from("The export lacks the declaration registry, parameter checks and declared universe used for inductive formation") }),
        spec("K_Pos", Inductive, 0, true, ModelUnavailable { reason: Text::from("The export lacks the checked strict-positivity judgment for constructor argument types") }),
        spec("K_Elim", Inductive, 3, false, ModelUnavailable { reason: Text::from("The export lacks the registered declaration and the typing judgment for every constructor branch") }),
        spec("K_Smt", SmtAxiom, 0, true, Unsupported {
                reason: Text::from(
                    "Opaque SMT envelopes have no checked reconstruction and are refused by the executable kernel",
                ),
            }),
        spec("K_FwAx", SmtAxiom, 0, true, ModelUnavailable { reason: Text::from("The export lacks the authoritative axiom registry lookup and coordinate policy used by the executable kernel") }),
        spec("K_Eps_Mu", Diakrisis, 2, false, Unsupported { reason: Text::from("No executable rule authorizes assigning an arbitrary articulation the type of an enactment; the biadjunction correspondence remains unverified") }),
        spec("K_Universe_Ascent", Diakrisis, 1, true, Proved),
        spec("K_Round_Trip", Diakrisis, 2, false, Unsupported { reason: Text::from("No executable rule authorizes assigning an arbitrary term a recovered type; an encoding round-trip requires checked source correspondence") }),
        spec("K_Epsilon_Of", Diakrisis, 1, false, Proved),
        spec("K_Alpha_Of", Diakrisis, 1, false, Proved),
        spec("K_Modal_Box", Diakrisis, 1, false, Proved),
        spec("K_Modal_Diamond", Diakrisis, 1, false, Proved),
        spec("K_Modal_Big_And", Diakrisis, 1, false, Proved),
        spec("K_Shape", Diakrisis, 1, false, Proved),
        spec("K_Flat", Diakrisis, 1, false, Proved),
        spec("K_Sharp", Diakrisis, 1, false, Proved),
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

    /// Cubical Agda syntax token for this arg type.  `String` and
    /// `ℕ` are Agda primitives; `CoreTerm` is the inductive declared
    /// at the top of the Agda export alongside `Ctx` and `KernelRule`.
    pub fn agda_repr(self) -> &'static str {
        match self {
            IouArgType::CoreTerm => "CoreTerm",
            IouArgType::Str => "String",
            IouArgType::Nat => "ℕ",
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
/// This registry is empty. An unresolved claim is recorded in the catalogue
/// and omitted from proof export. A framework citation never creates an axiom.
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

/// Render the IOU axiom block for Cubical Agda: `postulate
/// <Name>_iou : Ctx → <type1> → … → <typeN> → Set` with leading
/// citation comment.  Mirrors `render_iou_axioms_lean` /
/// `_coq` / `_isabelle` so the four foundations stay in
/// lockstep — every entry of `iou_axiom_specs()` materialises
/// the same trust-extension surface in Agda's syntax.
pub fn render_iou_axioms_agda() -> String {
    let specs = iou_axiom_specs();
    let mut out = String::new();
    out.push_str(&format!(
        "-- ====== Per-rule IOU axioms ({} total) ======\n\
         -- Each captures a meta-theory dependency not yet formalised in\n\
         -- Cubical Agda; discharging an IOU = replacing the postulate\n\
         -- with a real definition (or folding its content into\n\
         -- structural premises of the corresponding `Typing` rule).\n",
        specs.len()
    ));
    if !specs.is_empty() {
        out.push_str("postulate\n");
        for spec in &specs {
            out.push_str(&format!("  -- {}: {}.\n", spec.rule_name, spec.comment));
            out.push_str(&format!("  {}_iou : Ctx", spec.rule_name));
            for arg in spec.arg_types {
                out.push_str(&format!(" → {}", arg.agda_repr()));
            }
            out.push_str(" → Set\n");
        }
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

/// What the selected backend emits for this rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaEvidence {
    /// A derivation in this backend’s typing relation.
    TypingDerivation,
    /// A rule name with no theorem in this backend.
    CatalogueOnly,
    /// This backend has no such rule in its model.
    Unavailable,
}

/// Renders a backend model with an explicit per-rule evidence scope.
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

    /// A schema derivation, a name in the inventory, or an explicit omission.
    fn schema_evidence(&self, rule: &RuleSpec) -> SchemaEvidence;

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

    /// Render the `Inductive KernelRule := …` block. The available
    /// entries appear in canonical order.
    fn render_kernel_rule_inductive(&self, rules: &[RuleSpec]) -> String;

    /// Render a single per-rule soundness lemma (statement + proof
    /// or an explanation of its omission).
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
        // Aggregate proofs quantify only over this explicit subset.
        // Unsupported rules are neither axioms nor vacuous proof premises.
        let supported: List<_> = self
            .rules
            .iter()
            .filter(|rule| backend.schema_evidence(rule) != SchemaEvidence::Unavailable)
            .cloned()
            .collect();
        let mut out = String::new();
        out.push_str(&backend.render_preamble());
        out.push_str("\n\n");
        out.push_str(&backend.render_core_term_inductive());
        out.push_str("\n\n");
        out.push_str(&backend.render_core_type_inductive());
        out.push_str("\n\n");
        out.push_str(&backend.render_kernel_rule_inductive(supported.as_slice()));
        out.push_str("\n\n");
        for rule in &self.rules {
            out.push_str(&backend.render_rule_lemma(rule));
            out.push_str("\n\n");
        }
        out.push_str(&backend.render_main_theorem(supported.as_slice()));
        out.push_str("\n\n");
        out.push_str(&backend.render_postscript());
        out.push('\n');
        out
    }

    /// Count derivations in this backend's model, excluding inventory entries.
    pub fn schema_derivation_count(&self, backend: &dyn SoundnessBackend) -> usize {
        self.rules
            .iter()
            .filter(|rule| backend.schema_evidence(rule) == SchemaEvidence::TypingDerivation)
            .count()
    }

    /// Verify the catalogue size, supported schemas and absence of implicit authority.
    pub fn drift_check(&self) -> Result<(), String> {
        if self.rules.len() != EXPECTED_KERNEL_RULE_COUNT {
            return Err(format!(
                "kernel-soundness corpus has {} rules, expected {} \
                 — Rust enum and Verum corpus drift",
                self.rules.len(),
                EXPECTED_KERNEL_RULE_COUNT
            ));
        }

        for rule in &self.rules {
            if rule.status.is_supported()
                && (lean::LeanBackend.schema_evidence(rule) != SchemaEvidence::TypingDerivation
                    || coq::CoqBackend.schema_evidence(rule) != SchemaEvidence::TypingDerivation)
            {
                return Err(format!("missing checked schema for {}", rule.rule_name));
            }
        }
        if !iou_axiom_specs().is_empty() {
            return Err("unresolved claims must be omitted from the exported judgments".into());
        }
        Ok(())
    }

    /// Only explicit open claims belong on the admitted list.
    pub fn admitted_iou_list(&self) -> Vec<(&str, &str)> {
        self.rules
            .iter()
            .filter_map(|rule| match &rule.status {
                LemmaStatus::Admitted { reason } => {
                    Some((rule.rule_name.as_str(), reason.as_str()))
                }
                _ => None,
            })
            .collect()
    }

    /// Project: count of lemmas in `Proved` status.
    pub fn proved_count(&self) -> usize {
        self.rules.iter().filter(|r| r.status.is_proved()).count()
    }

    /// Rules explicitly refused by the executable kernel and omitted from
    /// foreign admission schemas. These never count as discharged proofs.
    pub fn unsupported_count(&self) -> usize {
        self.rules
            .iter()
            .filter(|rule| matches!(rule.status, LemmaStatus::Unsupported { .. }))
            .count()
    }

    /// Count native rules for which the export lacks a required model.
    pub fn model_unavailable_count(&self) -> usize {
        self.rules
            .iter()
            .filter(|rule| matches!(rule.status, LemmaStatus::ModelUnavailable { .. }))
            .count()
    }

    /// Whether every catalogue entry has an exported schema. This does not
    /// establish correspondence with the executable checker or soundness.
    pub fn has_complete_schema_coverage(&self) -> bool {
        self.rules.iter().all(|rule| rule.status.is_supported())
    }

    /// Project: count of lemmas in `Admitted` status (open IOU only,
    /// excludes `UnverifiedFrameworkReference`).
    pub fn admitted_count(&self) -> usize {
        self.rules
            .iter()
            .filter(|r| matches!(r.status, LemmaStatus::Admitted { .. }))
            .count()
    }

    /// Count unresolved citations separately from the schema derivations.
    pub fn unverified_framework_reference_count(&self) -> usize {
        self.rules
            .iter()
            .filter(|r| r.status.is_unverified_framework_reference())
            .count()
    }
}

impl Default for SoundnessExporter {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Trust-extension report
// =============================================================================

/// One entry in the per-rule trust-extension report.  Everything an
/// auditor needs to follow a single rule's trust extension surface
/// is bundled here: status, premises, citation, per-foundation
/// lemma signature shape (so the auditor can spot-check that the
/// emitted Lean / Coq / Isabelle file's rendered lemma matches the
/// claimed shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustExtensionEntry {
    /// Stable rule name in `K_<Name>` form.
    pub rule_name: String,
    /// Stable lemma name in `K_<Name>_sound` form.
    pub lemma_name: String,
    /// Coarse category for grouping (Structural / Cubical / Refinement
    /// / Quotient / Inductive / SmtAxiom / Diakrisis).
    pub category: String,
    /// Number of premise sub-derivations the rule's Typing constructor
    /// expects.
    pub premise_arity: usize,
    /// Whether the rule has a non-judgmental side-condition obligation.
    pub has_side_condition: bool,
    /// Status kind as a stable string: `"proved"` / `"unsupported"` /
    /// `"model_unavailable"` / `"admitted"` /
    /// `"unverified_framework_reference"`.  Stable across runs; consumed by
    /// machine readers.
    pub status_kind: String,
    /// For `Admitted`: the stated reason.  For
    /// `UnverifiedFrameworkReference`: the citation text.  `None` for
    /// `Proved`.
    pub trust_note: Option<String>,
    /// For `UnverifiedFrameworkReference`: the cited upstream lemma path.
    /// `None` otherwise.
    pub framework_lemma_path: Option<String>,
    /// For `UnverifiedFrameworkReference`: the framework name (`mathlib4` /
    /// `category-theory` / `verum-internal` / `zfc` / …).  `None`
    /// otherwise.
    pub framework_name: Option<String>,
}

/// Aggregate trust-extension report.  Top-level partition of the
/// corpus + the per-rule entries.  Serialised as JSON via the audit
/// gate; the human-readable rendering is built from the same
/// instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustExtensionReport {
    /// Total number of kernel rules in the corpus.  Should always
    /// equal [`EXPECTED_KERNEL_RULE_COUNT`] (currently 38); pin-tested.
    pub total_rules: usize,
    /// Count of rules in `Proved` status — structurally typed in
    /// the inductive without any framework citation.
    pub proved_count: usize,
    /// Refused rules with no exported admission or soundness lemma.
    pub unsupported_count: usize,
    /// Rules whose native authority is outside the exported model.
    pub model_unavailable_count: usize,
    /// Count of rules in `Admitted` status — open IOUs.  After
    /// FV-17 this is `0`; non-zero means an open IOU has been
    /// re-introduced.
    pub admitted_count: usize,
    /// References whose metatheory and correspondence have not been checked.
    pub unverified_framework_reference_count: usize,
    /// Length of [`iou_axiom_specs`] — the count of `axiom
    /// K_<rule>_iou` declarations emitted into the per-foundation
    /// files.  Post-FV-17 this is `0`.
    pub open_iou_axioms: usize,
    /// Per-rule entries in canonical KernelRule order.
    pub entries: Vec<TrustExtensionEntry>,
    /// Top-level prose summary for human consumption.  Stable
    /// across runs barring corpus changes; not consumed by machine
    /// readers (use the structured fields above).
    pub summary: String,
}

impl SoundnessExporter {
    /// Render the trust-extension report — auditor-facing canonical
    /// snapshot of the kernel-soundness corpus's trust extension
    /// surface.  Walks [`canonical_rules`] and produces one
    /// [`TrustExtensionEntry`] per rule plus aggregate counts.
    ///
    /// **Invariants** (pin-tested):
    /// * `total_rules == EXPECTED_KERNEL_RULE_COUNT`.
    /// * All five mutually exclusive statuses partition the rule catalogue.
    /// * `open_iou_axioms == iou_axiom_specs().len()` (post-FV-17 this
    ///   is `0`).
    pub fn render_trust_extension_report(&self) -> TrustExtensionReport {
        let mut entries = Vec::with_capacity(self.rules.len());
        for rule in &self.rules {
            let category = match rule.category {
                RuleCategory::Structural => "Structural",
                RuleCategory::Cubical => "Cubical",
                RuleCategory::Refinement => "Refinement",
                RuleCategory::Quotient => "Quotient",
                RuleCategory::Inductive => "Inductive",
                RuleCategory::SmtAxiom => "SmtAxiom",
                RuleCategory::Diakrisis => "Diakrisis",
            };
            let (status_kind, trust_note, framework_lemma_path, framework_name) = match &rule.status
            {
                LemmaStatus::Unsupported { reason } => {
                    ("unsupported", Some(reason.to_string()), None, None)
                }
                LemmaStatus::ModelUnavailable { reason } => ("model_unavailable", Some(reason.to_string()), None, None),
                LemmaStatus::Proved => ("proved", None, None, None),
                LemmaStatus::Admitted { reason } => ("admitted", Some(reason.clone()), None, None),
                LemmaStatus::UnverifiedFrameworkReference {
                    lemma_path,
                    framework,
                    citation,
                } => (
                    "unverified_framework_reference",
                    Some(citation.clone()),
                    Some(lemma_path.clone()),
                    Some(framework.clone()),
                ),
            };
            entries.push(TrustExtensionEntry {
                rule_name: rule.rule_name.clone(),
                lemma_name: rule.lemma_name.clone(),
                category: category.to_string(),
                premise_arity: rule.premise_arity,
                has_side_condition: rule.has_side_condition,
                status_kind: status_kind.to_string(),
                trust_note,
                framework_lemma_path,
                framework_name,
            });
        }

        let proved = self.proved_count();
        let unsupported = self.unsupported_count();
        let admitted = self.admitted_count();
        let references = self.unverified_framework_reference_count();
        let model_unavailable = self.model_unavailable_count();
        let open_iou_axioms = iou_axiom_specs().len();

        let summary = format!(
            "The abstract catalogue contains {total} rules. Lean and Coq derive {schemas} typing schemas, \
             including {references} with unverified metatheory references. Isabelle derives {structural} structural \
             schemas. Agda checks syntax and rule names. The catalogue records {unsupported} unsupported rules, \
             {model_unavailable} unavailable models and {admitted} unresolved claims. \
             Correspondence with the executable checker remains unproved.",
            total = self.rules.len(),
            schemas = self.schema_derivation_count(&lean::LeanBackend),
            structural = self.schema_derivation_count(&isabelle::IsabelleBackend),
        );

        TrustExtensionReport {
            total_rules: self.rules.len(),
            proved_count: proved,
            unsupported_count: unsupported,
            model_unavailable_count: model_unavailable,
            admitted_count: admitted,
            unverified_framework_reference_count: references,
            open_iou_axioms,
            entries,
            summary,
        }
    }
}
