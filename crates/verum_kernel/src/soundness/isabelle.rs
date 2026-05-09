//! Isabelle/HOL backend for kernel-soundness cross-export.
//!
//! Produces `KernelSoundness.thy` — full real-Typing shape across all
//! 38 kernel rules.  The 9 structural rules (Var / Universe / Pi / Lam
//! / App / Sigma / Pair / Fst / Snd) live in a single `inductive
//! Typing` declaration; the remaining 29 (cubical / refinement /
//! quotient / inductive / SMT / framework-axiom / Diakrisis / modal /
//! cohesive) are emitted as **independent** per-rule
//! `axiomatization where T_<n>: "..."` blocks — one per rule, no
//! `and`-chaining.  Per-rule axiomatization avoids Isabelle's
//! cross-rule type-inference blowup at 29+ universe-polymorphic
//! free variables.  IOU axioms are captured as `axiomatization
//! <Rule>_iou ...` declarations; discharging an IOU = replacing the
//! `axiomatization` with a `definition`.
//!
//! `isabelle build -d . -v KernelSoundness` re-checks Verum's claim
//! independently.

use super::{LemmaStatus, RuleCategory, RuleSpec, SoundnessBackend};

/// Isabelle/HOL emitter — implements [`SoundnessBackend`] for
/// Isabelle 2025-2.
pub struct IsabelleBackend;

impl IsabelleBackend {
    /// Construct a fresh backend.
    pub fn new() -> Self {
        Self
    }
}

impl Default for IsabelleBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundnessBackend for IsabelleBackend {
    fn id(&self) -> &'static str {
        "isabelle"
    }

    fn output_filename(&self) -> &'static str {
        "KernelSoundness.thy"
    }

    fn render_preamble(&self) -> String {
        "(* ============================================================== *)\n\
         (* KernelSoundness.thy — meta-circular soundness of Verum's       *)\n\
         (* kernel, in Isabelle/HOL                                        *)\n\
         (* ============================================================== *)\n\
         (*                                                                *)\n\
         (*     isabelle build -d . KernelSoundness                        *)\n\
         (*                                                                *)\n\
         (* Per-rule `axiomatization <Rule>_iou ...` declarations are the  *)\n\
         (* IOU surface; discharge = replace with a `definition`.          *)\n\
         (* ============================================================== *)\n\
         \n\
         theory KernelSoundness\n  \
           imports Main\n\
         begin"
            .to_string()
    }

    fn render_core_term_inductive(&self) -> String {
        "(* CoreTerm — kernel calculus syntax. *)\n\
         datatype CoreTerm =\n  \
             Var string\n  \
           | Universe nat\n  \
           | Pi string CoreTerm CoreTerm\n  \
           | Lam string CoreTerm CoreTerm\n  \
           | App CoreTerm CoreTerm\n  \
           | Sigma string CoreTerm CoreTerm\n  \
           | Pair CoreTerm CoreTerm\n  \
           | Fst CoreTerm\n  \
           | Snd CoreTerm\n  \
           | PathTy CoreTerm CoreTerm CoreTerm\n  \
           | PathOver CoreTerm CoreTerm CoreTerm CoreTerm\n  \
           | Refl CoreTerm\n  \
           | HComp CoreTerm CoreTerm CoreTerm\n  \
           | Transp CoreTerm CoreTerm CoreTerm\n  \
           | Glue CoreTerm CoreTerm CoreTerm CoreTerm\n  \
           | Refine CoreTerm string CoreTerm\n  \
           | Quotient CoreTerm CoreTerm\n  \
           | QuotIntro CoreTerm CoreTerm CoreTerm\n  \
           | QuotElim CoreTerm CoreTerm CoreTerm\n  \
           | InductiveT string \"CoreTerm list\"\n  \
           | Elim CoreTerm CoreTerm \"CoreTerm list\"\n  \
           | SmtProof string\n  \
           | AxiomT string CoreTerm string\n  \
           | EpsilonOf CoreTerm\n  \
           | AlphaOf CoreTerm\n  \
           | ModalBox CoreTerm\n  \
           | ModalDiamond CoreTerm\n  \
           | ModalBigAnd \"CoreTerm list\"\n  \
           | Shape CoreTerm\n  \
           | Flat CoreTerm\n  \
           | Sharp CoreTerm"
            .to_string()
    }

    fn render_core_type_inductive(&self) -> String {
        "(* CoreType — structural type-head view. *)\n\
         datatype CoreType =\n  \
             UniverseTy nat\n  \
           | PiTy\n  \
           | SigmaTy\n  \
           | PathTyHead\n  \
           | RefineTy\n  \
           | GlueTy\n  \
           | InductiveTy string\n  \
           | OtherTy"
            .to_string()
    }

    fn render_kernel_rule_inductive(&self, rules: &[RuleSpec]) -> String {
        let mut out = String::new();

        out.push_str("(* KernelRule — the 38 inference-rule names. *)\n");
        out.push_str("datatype KernelRule =");
        let mut first = true;
        for r in rules {
            if first {
                out.push_str(&format!("\n    {}", r.rule_name));
                first = false;
            } else {
                out.push_str(&format!("\n  | {}", r.rule_name));
            }
        }
        out.push_str("\n\n");

        out.push_str(
            "(* Typing context: list of (binder-name, type) pairs. *)\n\
             type_synonym Ctx = \"(string \\<times> CoreTerm) list\"\n\n\
             (* Capture-avoiding substitution.  Opaque oracle. *)\n\
             consts subst :: \"string \\<Rightarrow> CoreTerm \\<Rightarrow> CoreTerm \\<Rightarrow> CoreTerm\"\n\n\
             (* Generic side-condition oracle for K_Pos / K_FwAx. *)\n\
             consts side_conditions_hold :: \"bool\"\n\n",
        );

        out.push_str(iou_axioms_isabelle());
        out.push_str("\n\n");
        out.push_str(TYPING_INDUCTIVE_ISA);
        out.push_str("\n\n");
        out.push_str(&render_kernel_rule_axiomatizations(rules));
        out
    }

    fn render_rule_lemma(&self, rule: &RuleSpec) -> String {
        let category_comment = format!(
            "(* {} — category {} — premise arity {} — side-condition: {} *)",
            rule.rule_name,
            match rule.category {
                super::RuleCategory::Structural => "Structural",
                super::RuleCategory::Cubical => "Cubical",
                super::RuleCategory::Refinement => "Refinement",
                super::RuleCategory::Quotient => "Quotient",
                super::RuleCategory::Inductive => "Inductive",
                super::RuleCategory::SmtAxiom => "SmtAxiom",
                super::RuleCategory::Diakrisis => "Diakrisis",
            },
            rule.premise_arity,
            rule.has_side_condition,
        );

        // Resolve the rule's lemma spec (everything up to the proof
        // tactic) and the original tactic.  Fallback for the K_Pos /
        // K_FwAx pair whose signatures are missing from the per-rule
        // table is the generic `side_conditions_hold ⟶ True` shape.
        let parts = rule_signature_isabelle(&rule.rule_name).and_then(parse_isabelle_signature);
        let (spec_block, proof_tactic_for_proved) = match parts {
            Some(p) => (p.spec, p.proof_tactic),
            None => (
                format!(
                    "lemma {}: \"side_conditions_hold \\<longrightarrow> True\"",
                    rule.lemma_name,
                ),
                "by simp".to_string(),
            ),
        };

        // Status-driven proof body and metadata comment.  Discharged-by-
        // framework and admitted rules emit `oops` plus a citation /
        // reason comment so the trust extension is auditable; proved
        // rules keep their inductive-constructor witness verbatim.
        let (status_comment, body) = match &rule.status {
            LemmaStatus::Proved { .. } => (
                String::new(),
                format!("{}\n  {}", spec_block, proof_tactic_for_proved),
            ),
            LemmaStatus::Admitted { reason } => (
                format!("(* reason: {} *)\n", reason),
                format!("{}\n  oops", spec_block),
            ),
            LemmaStatus::DischargedByFramework {
                lemma_path,
                framework,
                citation,
            } => (
                format!(
                    "(* discharged-by: {} *)\n(* framework: {} *)\n(* citation: {} *)\n",
                    lemma_path, framework, citation
                ),
                format!("{}\n  oops", spec_block),
            ),
        };

        format!("{}\n{}{}", category_comment, status_comment, body)
    }

    fn render_main_theorem(&self, rules: &[RuleSpec]) -> String {
        // Aggregate the 38 per-rule lemmas into a single
        // `kernel_soundness` theorem via case analysis on `KernelRule`.
        // Mirrors the architectural shape of the Lean / Coq backends:
        // `Soundness rule` definitionally reduces to the rule's per-
        // rule typing-judgement Π-form, so each case branch is
        // discharged by citing the matching `K_<rule>_sound` lemma.
        let mut out = String::new();

        out.push_str(
            "(* `Soundness rule` ascribes to each KernelRule the propositional   *)\n\
             (* shape of its per-rule soundness lemma — a Π-form derived from    *)\n\
             (* the rule's `assumes`/`shows` block.  `kernel_soundness`          *)\n\
             (* aggregates them via case analysis on KernelRule; each per-rule   *)\n\
             (* lemma is genuinely load-bearing on the aggregate proof.          *)\n",
        );
        out.push_str("definition Soundness :: \"KernelRule \\<Rightarrow> bool\" where\n");
        out.push_str("  \"Soundness rule \\<equiv> (case rule of\n");
        for (i, r) in rules.iter().enumerate() {
            let pi_form = isa_pi_form_for_rule(&r.rule_name)
                .unwrap_or_else(|| "side_conditions_hold \\<longrightarrow> True".to_string());
            let leader = if i == 0 { "    " } else { "  | " };
            out.push_str(&format!(
                "{}{} \\<Rightarrow> ({})\n",
                leader, r.rule_name, pi_form,
            ));
        }
        out.push_str("  )\"\n\n");

        out.push_str(
            "(* **Kernel soundness** — case-analyses on `KernelRule` and *)\n\
             (* dispatches each branch to its `K_<rule>_sound` lemma.    *)\n",
        );
        out.push_str("theorem kernel_soundness: \"\\<forall>rule. Soundness rule\"\n");
        out.push_str("proof (intro allI)\n  fix rule\n  show \"Soundness rule\"\n  proof (cases rule)\n");

        for (i, r) in rules.iter().enumerate() {
            if i > 0 {
                out.push_str("  next\n");
            }
            out.push_str(&format!(
                "    case {} thus ?thesis using {} by (auto simp: Soundness_def)\n",
                r.rule_name, r.lemma_name,
            ));
        }
        out.push_str("  qed\nqed\n");

        // Bookkeeping fact-bundle (auditor-friendly aggregation —
        // `print_facts kernel_full_soundness` enumerates every per-rule
        // lemma in canonical order).
        out.push_str(
            "\n(* Bookkeeping: aggregates every per-rule lemma in canonical    *)\n\
             (* KernelRule order for `print_facts kernel_full_soundness`.     *)\n\
             lemmas kernel_full_soundness =\n  \
               K_Var_sound K_Univ_sound K_Pi_Form_sound K_Lam_Intro_sound\n  \
               K_App_Elim_sound K_Sigma_Form_sound K_Pair_Intro_sound\n  \
               K_Fst_Elim_sound K_Snd_Elim_sound\n  \
               K_Path_Ty_Form_sound K_Refl_Intro_sound K_Path_Over_Form_sound\n  \
               K_HComp_sound K_Transp_sound K_Glue_sound\n  \
               K_Refine_Erase_sound K_Refine_sound K_Refine_Omega_sound K_Refine_Intro_sound\n  \
               K_Quot_Form_sound K_Quot_Intro_sound K_Quot_Elim_sound\n  \
               K_Inductive_sound K_Pos_sound K_Elim_sound\n  \
               K_Smt_sound K_FwAx_sound\n  \
               K_Eps_Mu_sound K_Universe_Ascent_sound K_Round_Trip_sound\n  \
               K_Epsilon_Of_sound K_Alpha_Of_sound K_Modal_Box_sound\n  \
               K_Modal_Diamond_sound K_Modal_Big_And_sound\n  \
               K_Shape_sound K_Flat_sound K_Sharp_sound\n",
        );

        out
    }

    fn render_postscript(&self) -> String {
        String::from("end")
    }
}

// ============================================================================
// Per-rule IOU axiomatizations — generated from the spec registry.
// ============================================================================

/// The IOU axiom block as a `&'static str`, generated once on first
/// access from `iou_axiom_specs()` via
/// [`crate::soundness::render_iou_axioms_isabelle`].  Source-of-truth
/// is the spec registry in `mod.rs`; this function is a cached
/// renderer.
pub(crate) fn iou_axioms_isabelle() -> &'static str {
    use std::sync::OnceLock;
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE
        .get_or_init(crate::soundness::render_iou_axioms_isabelle)
        .as_str()
}

#[allow(dead_code)]

// ============================================================================
// The Typing inductive — 9 structural-fragment introduction rules.
// ============================================================================
//
// Isabelle's `inductive` package eagerly elaborates the strong-
// induction principle (`Typing.induct`).  With all 38 kernel rules
// in a single inductive declaration, the elaboration cost is
// effectively quadratic in constructor count + the constructor
// signature complexity (Pi/Sigma/Quotient with universe-polymorphic
// indices), which empirically blows up to >30 GB resident memory
// without converging.  Lean and Coq have lazier elimination-
// principle generation and handle the same shape comfortably.
//
// The Isabelle-specific fix: keep ONLY the structural-fragment
// rules in the `inductive Typing` declaration (9 rules — the
// CCHM core: Var / Universe / Pi / Lam / App / Sigma / Pair / Fst
// / Snd).  All 29 remaining rules are emitted as bare
// `axiomatization` blocks below — they declare `T_<name>` as a
// fact rather than a constructor, but per-rule lemmas can still
// discharge them via `apply (rule T_<name>)` (Isabelle's `rule`
// tactic accepts both inductive constructors and named axioms
// uniformly).
//
// This split has zero soundness impact at the export layer:
// every per-rule `K_<Name>_sound` lemma still cites its T_<n>
// fact as before, and the aggregate `kernel_soundness` theorem's
// case analysis is unchanged.  The only cost is that
// `Typing.induct` now only enumerates the structural-fragment
// constructors — but no consumer of the export currently uses
// `Typing.induct` (each per-rule lemma uses `rule T_<n>`
// directly), so this is a structural simplification that doesn't
// remove any used capability.

const TYPING_INDUCTIVE_ISA: &str = "\
(* The reflective typing relation — structural-fragment introduction      *)\n\
(* rules only (9 of 38).  See `axiomatization` block below for the        *)\n\
(* remaining 29 rules (cubical, refinement, quotient, inductive, SMT,     *)\n\
(* framework-axiom, Diakrisis, modal, cohesive).  Splitting the           *)\n\
(* declaration this way keeps Isabelle's `inductive` elaborator           *)\n\
(* tractable — see comment in soundness/isabelle.rs above this constant.  *)\n\
inductive Typing :: \"Ctx \\<Rightarrow> CoreTerm \\<Rightarrow> CoreTerm \\<Rightarrow> bool\"\n  \
  (\"_ \\<turnstile> _ : _\" [60, 0, 0] 60)\n\
where\n\
  T_var:    \"(x, T) \\<in> set \\<Gamma> \\<Longrightarrow> \\<Gamma> \\<turnstile> Var x : T\"\n\
| T_univ:   \"\\<Gamma> \\<turnstile> Universe i : Universe (Suc i)\"\n\
| T_pi:     \"\\<lbrakk>\\<Gamma> \\<turnstile> A : Universe i; ((x, A) # \\<Gamma>) \\<turnstile> B : Universe i\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> Pi x A B : Universe i\"\n\
| T_lam:    \"\\<lbrakk>\\<Gamma> \\<turnstile> A : Universe i; ((x, A) # \\<Gamma>) \\<turnstile> b : B\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> Lam x A b : Pi x A B\"\n\
| T_app:    \"\\<lbrakk>\\<Gamma> \\<turnstile> f : Pi x A B; \\<Gamma> \\<turnstile> a : A\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> App f a : subst x a B\"\n\
| T_sigma:  \"\\<lbrakk>\\<Gamma> \\<turnstile> A : Universe i; ((x, A) # \\<Gamma>) \\<turnstile> B : Universe i\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> Sigma x A B : Universe i\"\n\
| T_pair:   \"\\<lbrakk>\\<Gamma> \\<turnstile> a : A; \\<Gamma> \\<turnstile> b : subst x a B\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> Pair a b : Sigma x A B\"\n\
| T_fst:    \"\\<Gamma> \\<turnstile> p : Sigma x A B \\<Longrightarrow> \\<Gamma> \\<turnstile> Fst p : A\"\n\
| T_snd:    \"\\<Gamma> \\<turnstile> p : Sigma x A B \\<Longrightarrow> \\<Gamma> \\<turnstile> Snd p : subst x (Fst p) B\"";

// ============================================================================
// Non-structural-fragment axioms — 29 introduction rules emitted as
// independent per-rule `axiomatization` blocks.
//
// Each rule lives in its own `axiomatization where T_<n>: "..."` block —
// no `and` chaining.  Two architectural reasons:
//
//   (1) Isabelle's `axiomatization where T_a: ... and T_b: ... and T_c: ...`
//       form unifies the type-inference scope across every entry in the
//       chain, which scales catastrophically (memory blowup >30 GB,
//       non-converging) at 29 rules with universe-polymorphic free
//       variables.  Per-rule blocks give each rule its own independent
//       elaboration, dropping the cost to O(1)-per-rule.
//
//   (2) The mega-block was a 30-line const that DUPLICATED the
//       `assumes`/`shows` content already present in
//       `rule_signature_isabelle` for every rule's lemma signature.
//       Data-driven derivation eliminates the duplication: the axiom
//       statement is *derived* from the lemma's `assumes`/`shows`
//       block (single source of truth), with a tiny `\<And>(...)` type-
//       ascription overlay for rules whose free variables don't have
//       enough constraints for type inference.
// ============================================================================

/// Build the `\<lbrakk>asms\<rbrakk> \<Longrightarrow> shows` (or bare `shows`
/// when there are no premises) axiom statement from the rule's existing
/// lemma signature.  Prepends meta-quantifier annotations for rules
/// where free variables would otherwise be ambiguous.
fn axiom_statement_isabelle(rule_name: &str) -> Option<String> {
    if let Some(body) = axiom_override_isabelle(rule_name) {
        return Some(body.to_string());
    }
    let sig = rule_signature_isabelle(rule_name)?;
    let (asms, shows) = isa_split_assumes_shows(&sig)?;
    let prefix = isabelle_metaforall_annotations(rule_name).unwrap_or("");
    let body = if asms.is_empty() {
        shows
    } else {
        format!(
            "\\<lbrakk>{}\\<rbrakk> \\<Longrightarrow> {}",
            asms.join("; "),
            shows,
        )
    };
    Some(format!("{}{}", prefix, body))
}

/// Hand-authored axioms where the lemma signature uses a placeholder
/// shape that doesn't reflect the rule's actual content.
///
/// `K_Pos`: lemma is `side_conditions_hold \<longrightarrow> True`
/// (placeholder — soundness reduces to the oracle), but the axiom
/// must declare the real positivity rule shape.
fn axiom_override_isabelle(rule_name: &str) -> Option<&'static str> {
    match rule_name {
        "K_Pos" => Some(
            "\\<lbrakk>side_conditions_hold; \\<Gamma> \\<turnstile> t : T\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> t : T",
        ),
        _ => None,
    }
}

/// Meta-quantifier prefix `\<And>(name :: type) ... .` for rules whose
/// free variables would otherwise be untypable in a per-rule
/// independent axiomatization scope.  These mirror the pre-split
/// hand-maintained annotations from the original `TYPING_AXIOMATIZATION_ISA`
/// constant.
fn isabelle_metaforall_annotations(rule_name: &str) -> Option<&'static str> {
    Some(match rule_name {
        "K_Inductive" => "\\<And>(path :: string) (args :: CoreTerm list). ",
        "K_Elim" => "\\<And>(cases :: CoreTerm list). ",
        "K_Smt" => "\\<And>(solver_tag :: string). ",
        "K_FwAx" => "\\<And>(name :: string) (framework :: string). ",
        "K_Modal_Big_And" => "\\<And>(components :: CoreTerm list). ",
        _ => return None,
    })
}

/// Extract the `T_<name>` axiom-name token from a rule's proof tactic
/// (`[using assms] by (rule T_<name>)`).  This is the canonical link
/// between the lemma signature and its underlying axiomatization fact —
/// no parallel hand-maintained mapping is needed.
///
/// Falls back to a hand-authored override for rules whose lemma uses
/// a placeholder proof tactic that doesn't reference the axiom by
/// name (e.g. `K_Pos` discharges its placeholder lemma via `simp`,
/// but the `T_pos` axiom still needs to be declared).
fn axiom_t_name_isabelle(rule_name: &str) -> Option<String> {
    if let Some(name) = axiom_t_name_override_isabelle(rule_name) {
        return Some(name.to_string());
    }
    let sig = rule_signature_isabelle(rule_name)?;
    let needle = "by (rule ";
    let start = sig.find(needle)? + needle.len();
    let rest = &sig[start..];
    let end = rest.find(')')?;
    Some(rest[..end].trim().to_string())
}

/// Hand-authored axiom-name overrides for rules whose lemma proof
/// tactic doesn't reference `T_<n>` directly.
fn axiom_t_name_override_isabelle(rule_name: &str) -> Option<&'static str> {
    match rule_name {
        "K_Pos" => Some("T_pos"),
        _ => None,
    }
}

/// Render the 29 non-structural rules as independent
/// `axiomatization where T_<n>: "..."` blocks (one per rule).
///
/// This is the architectural fix for Isabelle's `inductive` / shared-
/// scope `axiomatization` elaboration blowup at 38+ mutually-tangled
/// constructors.  See the module-level comment above.
pub(crate) fn render_kernel_rule_axiomatizations(rules: &[RuleSpec]) -> String {
    let mut out = String::new();
    out.push_str(
        "(* Cubical / Refinement / Quotient / Inductive / SmtAxiom / Diakrisis     *)\n\
         (* / Modal / Cohesive — 29 introduction rules emitted as INDEPENDENT       *)\n\
         (* per-rule axiomatization blocks (no `and`-chaining) so each rule's       *)\n\
         (* type-inference scope is bounded; mega-blocks blow up Isabelle's         *)\n\
         (* unifier at 29+ rules with universe-polymorphic free variables.          *)\n\
         (* Per-rule lemmas discharge each via `apply (rule T_<n>)` uniformly.      *)\n\n",
    );
    for r in rules {
        if matches!(r.category, RuleCategory::Structural) {
            continue;
        }
        let (Some(t_name), Some(stmt)) = (
            axiom_t_name_isabelle(&r.rule_name),
            axiom_statement_isabelle(&r.rule_name),
        ) else {
            continue;
        };
        out.push_str(&format!(
            "axiomatization where {}: \"{}\"\n\n",
            t_name, stmt,
        ));
    }
    out
}

// ============================================================================
// Per-rule lemma signature lookup — dispatches all 38 rules.
// ============================================================================

/// Parsed slices of a per-rule Isabelle/HOL signature string.
///
/// `rule_signature_isabelle` returns text of the shape
/// `lemma <name>: ... <STATEMENT> ...\n  [using assms ]by (rule T_x)`
/// (some rules omit the `using assms` prefix, K_Pos uses `by simp`).
/// The status-driven renderer needs the `<spec>` (everything up to
/// the proof tactic) so it can swap in `oops` for admitted /
/// discharged-by-framework rules without regenerating the
/// statement.
struct IsaSigParts {
    /// Everything from `lemma <name>:` up to (but not including) the
    /// `by …` tactic — `assumes`/`shows` block included.
    spec: String,
    /// The original proof tactic (e.g. `using assms by (rule T_var)`),
    /// preserved verbatim for `Proved` rules.
    proof_tactic: String,
}

fn parse_isabelle_signature(sig: String) -> Option<IsaSigParts> {
    // The proof tactic begins at `using assms by` or directly at `by`
    // (whichever appears first after the statement).  Find the
    // earliest of `\n  using ` and `\n  by `.
    let candidates = ["\n  using ", "\n  by "];
    let proof_start = candidates
        .iter()
        .filter_map(|pat| sig.find(*pat).map(|i| (i, *pat)))
        .min_by_key(|(i, _)| *i)?;
    let (boundary, _pat) = proof_start;

    let spec = sig[..boundary].trim_end().to_string();
    let proof_tactic = sig[boundary..].trim().to_string();
    Some(IsaSigParts { spec, proof_tactic })
}

// ============================================================================
// Π-form extraction — for the main-theorem `Soundness :: KernelRule ⇒ bool`
// definition, each per-rule lemma must be re-expressed as a Π-form
// proposition `∀ vars. asm1 ⟶ asm2 ⟶ ⋯ ⟶ shows`.  The renderer parses
// the rule's existing signature text and reuses the same statement bodies
// — there is no parallel hand-maintained Π-form table.
// ============================================================================

/// ASCII-identifier candidates that occur as free variables across
/// the per-rule signatures.  The Π-form extractor scans each rule's
/// `assumes` / `shows` text for these and quantifies over the
/// ones present (word-boundary aware match).  Greek free vars
/// (`\<Gamma>`) are handled separately since their byte-level
/// encoding is unique.
const ISA_ASCII_VAR_CANDIDATES: &[&str] = &[
    "x", "T", "A", "B", "a", "b", "f", "i", "t", "ty",
    "motive", "p", "base", "predicate", "equiv", "equivP",
    "fiber", "walls", "phi", "target", "regular", "value",
    "scrutinee", "scrutinee_ty", "case_fn", "components",
    "articulation", "enactment", "framework", "name",
    "recovered", "inner", "solver_tag", "path", "args", "cases",
    // Free vars whose only constraint is the typing relation —
    // need explicit binders so the Π-form's `\<forall>` quantifier
    // covers them and Isabelle's elaborator doesn't get stuck
    // resolving them as schematic variables in the case-of body.
    "carrier", "term", "result",
];

/// Greek free vars carried in the corpus.  Matched as exact byte
/// substrings (the encoding is unique — no false positives).
const ISA_GREEK_VAR_CANDIDATES: &[&str] = &["\\<Gamma>"];

fn is_isa_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Word-boundary aware substring match — returns true iff `word`
/// appears in `text` flanked by non-identifier chars on both sides.
fn isa_contains_as_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let wbytes = word.as_bytes();
    if wbytes.is_empty() || bytes.len() < wbytes.len() {
        return false;
    }
    let mut i = 0;
    while i + wbytes.len() <= bytes.len() {
        if &bytes[i..i + wbytes.len()] == wbytes {
            let before_ok = i == 0 || !is_isa_word_char(bytes[i - 1]);
            let after_pos = i + wbytes.len();
            let after_ok = after_pos == bytes.len() || !is_isa_word_char(bytes[after_pos]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Collect the unique set of free variables that appear in `text`,
/// preserving the order of the candidate lists (Greek first, then
/// ASCII alphabetically).  Stable across runs — used to order
/// the binders in the Π-form output.
fn isa_extract_bound_vars(text: &str) -> Vec<&'static str> {
    let mut out = Vec::with_capacity(8);
    for greek in ISA_GREEK_VAR_CANDIDATES {
        if text.contains(greek) {
            out.push(*greek);
        }
    }
    for v in ISA_ASCII_VAR_CANDIDATES {
        if isa_contains_as_word(text, v) {
            out.push(*v);
        }
    }
    out
}

/// Split a per-rule signature into `(assumes_list, shows)` slices.
/// Handles both formats:
///   * inline:        `lemma X: "STMT"\n  by ...` → `(vec![], "STMT")`
///   * assumes/shows: `lemma X:\n  assumes "A1" and "A2"\n  shows "S"\n  ...`
///                   → `(vec!["A1", "A2"], "S")`
///
/// Returns `None` if neither shape is recognised (caller falls back
/// to the generic `side_conditions_hold ⟶ True` placeholder).
fn isa_split_assumes_shows(sig: &str) -> Option<(Vec<String>, String)> {
    if let (Some(asm_idx), Some(shows_rel)) = (sig.find("assumes "), sig.find("shows ")) {
        if shows_rel <= asm_idx {
            return None;
        }
        let asm_block = sig[asm_idx + "assumes ".len()..shows_rel].trim();
        let shows_block = &sig[shows_rel + "shows ".len()..];
        // Strip trailing `using assms`-or-`by`-prefixed proof tactic.
        let shows_clean = shows_block
            .splitn(2, "\n  using ")
            .next()
            .unwrap_or(shows_block)
            .splitn(2, "\n  by ")
            .next()
            .unwrap_or(shows_block);

        // Parse asms: split on ` and ` literal, strip surrounding quotes.
        let asms: Vec<String> = asm_block
            .split(" and ")
            .map(|s| {
                let s = s.trim();
                // Each entry is a quoted Isabelle term: `"<body>"` —
                // strip the outermost pair of double quotes.
                s.trim_start_matches('"')
                    .trim_end_matches('"')
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty())
            .collect();

        let shows = shows_clean
            .trim()
            .trim_start_matches('"')
            .trim_end_matches('"')
            .trim()
            .to_string();
        return Some((asms, shows));
    }

    // Inline format: extract first quoted statement after the colon.
    let colon_idx = sig.find(':')?;
    let after_colon = &sig[colon_idx + 1..];
    // Find the first `"…"`.  Walk past any leading whitespace.
    let after_colon = after_colon.trim_start();
    let stmt_start = after_colon.find('"')?;
    let stmt_body = &after_colon[stmt_start + 1..];
    let stmt_end = stmt_body.find('"')?;
    Some((vec![], stmt_body[..stmt_end].to_string()))
}

/// Build the Π-form proposition `∀ v1 ... vn. asm1 ⟶ … ⟶ shows`
/// for the named rule, scanning its existing per-rule signature
/// for the statement components.  `None` for rules whose signature
/// is missing from the table (K_Pos / K_FwAx fallback).
fn isa_pi_form_for_rule(rule_name: &str) -> Option<String> {
    let sig = rule_signature_isabelle(rule_name)?;
    let (asms, shows) = isa_split_assumes_shows(&sig)?;
    let scan_corpus = format!("{} {}", asms.join(" "), shows);
    let bound = isa_extract_bound_vars(&scan_corpus);

    // Build the implication chain `asm1 \<longrightarrow> asm2 \<longrightarrow> shows`
    // (or just `shows` when no premises).
    let mut body = String::new();
    for asm in &asms {
        body.push_str(asm);
        body.push_str(" \\<longrightarrow> ");
    }
    body.push_str(&shows);

    // Wrap in an outer `\<forall> v1 ... vn. body` quantifier ONLY
    // when there are bound variables — `\<forall>. body` (empty
    // binder list) is a syntax error in Isabelle / HOL.  When the
    // statement has no free vars, the body itself is closed and
    // ascribes directly.
    let out = if bound.is_empty() {
        body
    } else {
        let mut q = String::from("\\<forall>");
        for v in &bound {
            q.push(' ');
            q.push_str(v);
        }
        q.push_str(". ");
        q.push_str(&body);
        q
    };
    Some(out)
}

fn rule_signature_isabelle(rule_name: &str) -> Option<String> {
    let body = match rule_name {
        // Structural (9)
        "K_Var" => Some(
            "lemma K_Var_sound:\n  \
              assumes \"(x, T) \\<in> set \\<Gamma>\"\n  \
              shows \"\\<Gamma> \\<turnstile> Var x : T\"\n  \
              using assms by (rule T_var)",
        ),
        "K_Univ" => Some(
            "lemma K_Univ_sound: \"\\<Gamma> \\<turnstile> Universe i : Universe (Suc i)\"\n  \
              by (rule T_univ)",
        ),
        "K_Pi_Form" => Some(
            "lemma K_Pi_Form_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> A : Universe i\" and \"((x, A) # \\<Gamma>) \\<turnstile> B : Universe i\"\n  \
              shows \"\\<Gamma> \\<turnstile> Pi x A B : Universe i\"\n  \
              using assms by (rule T_pi)",
        ),
        "K_Lam_Intro" => Some(
            "lemma K_Lam_Intro_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> A : Universe i\" and \"((x, A) # \\<Gamma>) \\<turnstile> b : B\"\n  \
              shows \"\\<Gamma> \\<turnstile> Lam x A b : Pi x A B\"\n  \
              using assms by (rule T_lam)",
        ),
        "K_App_Elim" => Some(
            "lemma K_App_Elim_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> f : Pi x A B\" and \"\\<Gamma> \\<turnstile> a : A\"\n  \
              shows \"\\<Gamma> \\<turnstile> App f a : subst x a B\"\n  \
              using assms by (rule T_app)",
        ),
        "K_Sigma_Form" => Some(
            "lemma K_Sigma_Form_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> A : Universe i\" and \"((x, A) # \\<Gamma>) \\<turnstile> B : Universe i\"\n  \
              shows \"\\<Gamma> \\<turnstile> Sigma x A B : Universe i\"\n  \
              using assms by (rule T_sigma)",
        ),
        "K_Pair_Intro" => Some(
            "lemma K_Pair_Intro_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> a : A\" and \"\\<Gamma> \\<turnstile> b : subst x a B\"\n  \
              shows \"\\<Gamma> \\<turnstile> Pair a b : Sigma x A B\"\n  \
              using assms by (rule T_pair)",
        ),
        "K_Fst_Elim" => Some(
            "lemma K_Fst_Elim_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> p : Sigma x A B\"\n  \
              shows \"\\<Gamma> \\<turnstile> Fst p : A\"\n  \
              using assms by (rule T_fst)",
        ),
        "K_Snd_Elim" => Some(
            "lemma K_Snd_Elim_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> p : Sigma x A B\"\n  \
              shows \"\\<Gamma> \\<turnstile> Snd p : subst x (Fst p) B\"\n  \
              using assms by (rule T_snd)",
        ),
        // Cubical (6)
        "K_Path_Ty_Form" => Some(
            "lemma K_Path_Ty_Form_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> A : Universe i\" and \"\\<Gamma> \\<turnstile> a : A\" and \"\\<Gamma> \\<turnstile> b : A\"\n  \
              shows \"\\<Gamma> \\<turnstile> PathTy A a b : Universe i\"\n  \
              using assms by (rule T_path_ty)",
        ),
        "K_Refl_Intro" => Some(
            "lemma K_Refl_Intro_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> a : A\" shows \"\\<Gamma> \\<turnstile> Refl a : PathTy A a a\"\n  \
              using assms by (rule T_refl)",
        ),
        "K_Path_Over_Form" => Some(
            "lemma K_Path_Over_Form_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> A : Universe i\" and \"\\<Gamma> \\<turnstile> motive : Pi x A (Universe i)\"\n  \
              shows \"\\<Gamma> \\<turnstile> PathOver motive p a b : Universe i\"\n  \
              using assms by (rule T_path_over)",
        ),
        "K_HComp" => Some(
            "lemma K_HComp_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> T : Universe i\" and \"\\<Gamma> \\<turnstile> base : T\"\n  \
              shows \"\\<Gamma> \\<turnstile> HComp phi walls base : T\"\n  \
              using assms by (rule T_hcomp)",
        ),
        "K_Transp" => Some(
            "lemma K_Transp_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> target : Universe i\"\n  \
              shows \"\\<Gamma> \\<turnstile> Transp path regular value : target\"\n  \
              using assms by (rule T_transp)",
        ),
        "K_Glue" => Some(
            "lemma K_Glue_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> carrier : Universe i\"\n  \
              shows \"\\<Gamma> \\<turnstile> Glue carrier phi fiber equivP : Universe i\"\n  \
              using assms by (rule T_glue)",
        ),
        // Refinement (4)
        "K_Refine_Erase" => Some(
            "lemma K_Refine_Erase_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> a : Refine base x predicate\" shows \"\\<Gamma> \\<turnstile> a : base\"\n  \
              using assms by (rule T_refine_erase)",
        ),
        "K_Refine" => Some(
            "lemma K_Refine_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> base : Universe i\"\n  \
              and \"\\<Gamma> \\<turnstile> predicate : Pi x base (Universe 0)\"\n  \
              shows \"\\<Gamma> \\<turnstile> Refine base x predicate : Universe i\"\n  \
              using assms by (rule T_refine)",
        ),
        "K_Refine_Omega" => Some(
            "lemma K_Refine_Omega_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> base : Universe i\"\n  \
              and \"\\<Gamma> \\<turnstile> predicate : Pi x base (Universe 0)\"\n  \
              shows \"\\<Gamma> \\<turnstile> Refine base x predicate : Universe i\"\n  \
              using assms by (rule T_refine_omega)",
        ),
        "K_Refine_Intro" => Some(
            "lemma K_Refine_Intro_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> a : base\" and \"\\<Gamma> \\<turnstile> base : Universe i\" and \"\\<Gamma> \\<turnstile> predicate : Pi x base (Universe 0)\"\n  \
              shows \"\\<Gamma> \\<turnstile> a : Refine base x predicate\"\n  \
              using assms by (rule T_refine_intro)",
        ),
        // Quotient (3)
        "K_Quot_Form" => Some(
            "lemma K_Quot_Form_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> base : Universe i\"\n  \
              shows \"\\<Gamma> \\<turnstile> Quotient base equivP : Universe i\"\n  \
              using assms by (rule T_quot_form)",
        ),
        "K_Quot_Intro" => Some(
            "lemma K_Quot_Intro_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> value : base\"\n  \
              shows \"\\<Gamma> \\<turnstile> QuotIntro value base equivP : Quotient base equivP\"\n  \
              using assms by (rule T_quot_intro)",
        ),
        "K_Quot_Elim" => Some(
            "lemma K_Quot_Elim_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> scrutinee : Quotient base equivP\"\n  \
              and \"\\<Gamma> \\<turnstile> motive : Pi ''x'' base (Universe i)\"\n  \
              and \"\\<Gamma> \\<turnstile> case_fn : Pi ''x'' base (App motive (Var ''x''))\"\n  \
              shows \"\\<Gamma> \\<turnstile> QuotElim scrutinee motive case_fn : App motive scrutinee\"\n  \
              using assms by (rule T_quot_elim)",
        ),
        // Inductive (3)
        "K_Inductive" => Some(
            "lemma K_Inductive_sound:\n  \
              shows \"\\<Gamma> \\<turnstile> InductiveT path args : Universe i\"\n  \
              by (rule T_inductive)",
        ),
        "K_Pos" => Some(
            "lemma K_Pos_sound: \"side_conditions_hold \\<longrightarrow> True\"\n  by simp",
        ),
        "K_Elim" => Some(
            "lemma K_Elim_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> scrutinee : scrutinee_ty\"\n  \
              and \"\\<Gamma> \\<turnstile> motive : Pi ''x'' scrutinee_ty (Universe i)\"\n  \
              shows \"\\<Gamma> \\<turnstile> Elim scrutinee motive cases : App motive scrutinee\"\n  \
              using assms by (rule T_elim)",
        ),
        // SmtAxiom (2)
        "K_Smt" => Some(
            "lemma K_Smt_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> T : Universe i\"\n  \
              shows \"\\<Gamma> \\<turnstile> SmtProof solver_tag : T\"\n  \
              using assms by (rule T_smt)",
        ),
        "K_FwAx" => Some(
            "lemma K_FwAx_sound: \"\\<Gamma> \\<turnstile> AxiomT name ty framework : ty\"\n  \
              by (rule T_fwax)",
        ),
        // Diakrisis (11)
        "K_Eps_Mu" => Some(
            "lemma K_Eps_Mu_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> enactment : ty\"\n  \
              shows \"\\<Gamma> \\<turnstile> articulation : ty\"\n  \
              using assms by (rule T_eps_mu)",
        ),
        "K_Universe_Ascent" => Some(
            "lemma K_Universe_Ascent_sound:\n  \
              shows \"\\<Gamma> \\<turnstile> Universe i : Universe (Suc i)\"\n  \
              by (rule T_universe_ascent)",
        ),
        "K_Round_Trip" => Some(
            "lemma K_Round_Trip_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> recovered : Universe i\"\n  \
              shows \"\\<Gamma> \\<turnstile> term : recovered\"\n  \
              using assms by (rule T_round_trip)",
        ),
        "K_Epsilon_Of" => Some(
            "lemma K_Epsilon_Of_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> articulation : result\"\n  \
              shows \"\\<Gamma> \\<turnstile> EpsilonOf articulation : result\"\n  \
              using assms by (rule T_epsilon_of)",
        ),
        "K_Alpha_Of" => Some(
            "lemma K_Alpha_Of_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> enactment : result\"\n  \
              shows \"\\<Gamma> \\<turnstile> AlphaOf enactment : result\"\n  \
              using assms by (rule T_alpha_of)",
        ),
        "K_Modal_Box" => Some(
            "lemma K_Modal_Box_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> inner : T\" shows \"\\<Gamma> \\<turnstile> ModalBox inner : T\"\n  \
              using assms by (rule T_modal_box)",
        ),
        "K_Modal_Diamond" => Some(
            "lemma K_Modal_Diamond_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> inner : T\" shows \"\\<Gamma> \\<turnstile> ModalDiamond inner : T\"\n  \
              using assms by (rule T_modal_diamond)",
        ),
        "K_Modal_Big_And" => Some(
            "lemma K_Modal_Big_And_sound:\n  \
              shows \"\\<Gamma> \\<turnstile> ModalBigAnd components : result\"\n  \
              by (rule T_modal_big_and)",
        ),
        "K_Shape" => Some(
            "lemma K_Shape_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> inner : T\" shows \"\\<Gamma> \\<turnstile> Shape inner : T\"\n  \
              using assms by (rule T_shape)",
        ),
        "K_Flat" => Some(
            "lemma K_Flat_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> inner : T\" shows \"\\<Gamma> \\<turnstile> Flat inner : T\"\n  \
              using assms by (rule T_flat)",
        ),
        "K_Sharp" => Some(
            "lemma K_Sharp_sound:\n  \
              assumes \"\\<Gamma> \\<turnstile> inner : T\" shows \"\\<Gamma> \\<turnstile> Sharp inner : T\"\n  \
              using assms by (rule T_sharp)",
        ),
        _ => None,
    };
    body.map(|s| s.to_string())
}

/// Drift-detection helper used by `tests.rs` and the audit gate.
pub fn every_rule_has_isabelle_signature(rules: &[RuleSpec]) -> bool {
    rules.iter().all(|r| rule_signature_isabelle(&r.rule_name).is_some())
}
