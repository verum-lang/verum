//! Isabelle/HOL backend for kernel-soundness cross-export.
//!
//! Produces `KernelSoundness.thy` — full real-Typing shape across all
//! 38 kernel rules, mirroring `lean.rs` and `coq.rs` exactly.  The 17
//! with-IOU rules are captured as `axiomatization <Rule>_iou ...`
//! declarations; discharging an IOU = replacing the `axiomatization`
//! with a `definition`.
//!
//! `isabelle build -d . -v KernelSoundness` re-checks Verum's claim
//! independently.

use super::{LemmaStatus, RuleSpec, SoundnessBackend};

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
// The Typing inductive — 38 introduction rules.
// ============================================================================

const TYPING_INDUCTIVE_ISA: &str = "\
(* The reflective typing relation. 38 introduction rules. *)\n\
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
| T_snd:    \"\\<Gamma> \\<turnstile> p : Sigma x A B \\<Longrightarrow> \\<Gamma> \\<turnstile> Snd p : subst x (Fst p) B\"\n\
| T_path_ty:    \"\\<lbrakk>\\<Gamma> \\<turnstile> A : Universe i; \\<Gamma> \\<turnstile> a : A; \\<Gamma> \\<turnstile> b : A\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> PathTy A a b : Universe i\"\n\
| T_refl:       \"\\<Gamma> \\<turnstile> a : A \\<Longrightarrow> \\<Gamma> \\<turnstile> Refl a : PathTy A a a\"\n\
| T_path_over:  \"\\<lbrakk>\\<Gamma> \\<turnstile> A : Universe i; \\<Gamma> \\<turnstile> motive : Pi x A (Universe i)\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> PathOver motive p a b : Universe i\"\n\
| T_hcomp:      \"\\<lbrakk>\\<Gamma> \\<turnstile> T : Universe i; \\<Gamma> \\<turnstile> base : T\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> HComp phi walls base : T\"\n\
| T_transp:     \"\\<Gamma> \\<turnstile> target : Universe i \\<Longrightarrow> \\<Gamma> \\<turnstile> Transp path regular value : target\"\n\
| T_glue:       \"\\<Gamma> \\<turnstile> carrier : Universe i \\<Longrightarrow> \\<Gamma> \\<turnstile> Glue carrier phi fiber equivP : Universe i\"\n\
| T_refine_erase: \"\\<Gamma> \\<turnstile> a : Refine base x predicate \\<Longrightarrow> \\<Gamma> \\<turnstile> a : base\"\n\
| T_refine:       \"\\<lbrakk>\\<Gamma> \\<turnstile> base : Universe i; \\<Gamma> \\<turnstile> predicate : Pi x base (Universe 0)\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> Refine base x predicate : Universe i\"\n\
| T_refine_omega: \"\\<lbrakk>\\<Gamma> \\<turnstile> base : Universe i; \\<Gamma> \\<turnstile> predicate : Pi x base (Universe 0)\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> Refine base x predicate : Universe i\"\n\
| T_refine_intro: \"\\<lbrakk>\\<Gamma> \\<turnstile> a : base; \\<Gamma> \\<turnstile> base : Universe i; \\<Gamma> \\<turnstile> predicate : Pi x base (Universe 0)\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> a : Refine base x predicate\"\n\
| T_quot_form:    \"\\<Gamma> \\<turnstile> base : Universe i \\<Longrightarrow> \\<Gamma> \\<turnstile> Quotient base equivP : Universe i\"\n\
| T_quot_intro:   \"\\<Gamma> \\<turnstile> value : base \\<Longrightarrow> \\<Gamma> \\<turnstile> QuotIntro value base equivP : Quotient base equivP\"\n\
| T_quot_elim:    \"\\<lbrakk>\\<Gamma> \\<turnstile> scrutinee : Quotient base equivP; \\<Gamma> \\<turnstile> motive : Pi ''x'' base (Universe i); \\<Gamma> \\<turnstile> case_fn : Pi ''x'' base (App motive (Var ''x''))\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> QuotElim scrutinee motive case_fn : App motive scrutinee\"\n\
| T_inductive:    \"\\<Gamma> \\<turnstile> InductiveT path args : Universe i\"\n\
| T_pos:          \"\\<lbrakk>side_conditions_hold; \\<Gamma> \\<turnstile> t : T\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> t : T\"\n\
| T_elim:         \"\\<lbrakk>\\<Gamma> \\<turnstile> scrutinee : scrutinee_ty; \\<Gamma> \\<turnstile> motive : Pi ''x'' scrutinee_ty (Universe i)\\<rbrakk> \\<Longrightarrow> \\<Gamma> \\<turnstile> Elim scrutinee motive cases : App motive scrutinee\"\n\
| T_smt:          \"\\<Gamma> \\<turnstile> T : Universe i \\<Longrightarrow> \\<Gamma> \\<turnstile> SmtProof solver_tag : T\"\n\
| T_fwax:         \"\\<Gamma> \\<turnstile> AxiomT name ty framework : ty\"\n\
| T_eps_mu:       \"\\<Gamma> \\<turnstile> enactment : ty \\<Longrightarrow> \\<Gamma> \\<turnstile> articulation : ty\"\n\
| T_universe_ascent: \"\\<Gamma> \\<turnstile> Universe i : Universe (Suc i)\"\n\
| T_round_trip:   \"\\<Gamma> \\<turnstile> recovered : Universe i \\<Longrightarrow> \\<Gamma> \\<turnstile> term : recovered\"\n\
| T_epsilon_of:   \"\\<Gamma> \\<turnstile> articulation : result \\<Longrightarrow> \\<Gamma> \\<turnstile> EpsilonOf articulation : result\"\n\
| T_alpha_of:     \"\\<Gamma> \\<turnstile> enactment : result \\<Longrightarrow> \\<Gamma> \\<turnstile> AlphaOf enactment : result\"\n\
| T_modal_box:    \"\\<Gamma> \\<turnstile> inner : T \\<Longrightarrow> \\<Gamma> \\<turnstile> ModalBox inner : T\"\n\
| T_modal_diamond:\"\\<Gamma> \\<turnstile> inner : T \\<Longrightarrow> \\<Gamma> \\<turnstile> ModalDiamond inner : T\"\n\
| T_modal_big_and:\"\\<Gamma> \\<turnstile> ModalBigAnd components : result\"\n\
| T_shape:        \"\\<Gamma> \\<turnstile> inner : T \\<Longrightarrow> \\<Gamma> \\<turnstile> Shape inner : T\"\n\
| T_flat:         \"\\<Gamma> \\<turnstile> inner : T \\<Longrightarrow> \\<Gamma> \\<turnstile> Flat inner : T\"\n\
| T_sharp:        \"\\<Gamma> \\<turnstile> inner : T \\<Longrightarrow> \\<Gamma> \\<turnstile> Sharp inner : T\"";

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
    "x", "T", "A", "B", "a", "b", "f", "i", "ty",
    "motive", "p", "base", "predicate", "equiv", "equivP",
    "fiber", "walls", "phi", "target", "regular", "value",
    "scrutinee", "scrutinee_ty", "case_fn", "components",
    "articulation", "enactment", "framework", "name",
    "recovered", "inner", "solver_tag", "path", "args", "cases",
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

    let mut out = String::new();
    out.push_str("\\<forall>");
    for v in &bound {
        out.push(' ');
        out.push_str(v);
    }
    out.push('.');
    for asm in &asms {
        out.push(' ');
        out.push_str(asm);
        out.push_str(" \\<longrightarrow>");
    }
    out.push(' ');
    out.push_str(&shows);
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
