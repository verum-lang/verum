//! Isabelle/HOL derivations for the nine structural schemas. The inductive
//! relation is closed. No axiom extends it with additional typing judgments.

use super::{RuleSpec, SchemaEvidence, SoundnessBackend, TypingSchema};

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

    fn schema_evidence(&self, rule: &RuleSpec) -> SchemaEvidence {
        if schema(rule).is_some() {
            SchemaEvidence::TypingDerivation
        } else {
            SchemaEvidence::Unavailable
        }
    }

    fn output_filename(&self) -> &'static str {
        "KernelSoundness.thy"
    }

    fn render_preamble(&self) -> String {
        "(* KernelSoundness.thy. Structural typing schemas. Substitution is an abstract constant. *)\ntheory KernelSoundness\n  imports Main\nbegin".into()
    }

    fn render_core_term_inductive(&self) -> String {
        "(* CoreTerm — kernel calculus syntax. *)\n\
         datatype CoreTerm =\n  \
             Var string\n  \
           | Universe nat\n  \
           | PropSort\n  \
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
           | Transport CoreTerm CoreTerm\n  \
           | Glue CoreTerm CoreTerm CoreTerm CoreTerm\n  \
           | Refine CoreTerm string CoreTerm\n  \
           | Quotient CoreTerm CoreTerm\n  \
           | QuotIntro CoreTerm CoreTerm CoreTerm\n  \
           | QuotElim CoreTerm CoreTerm CoreTerm CoreTerm\n  \
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

        out.push_str("(* KernelRule — the supported inference-rule names. *)\n");
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
            r#"type_synonym Ctx = "(string \<times> CoreTerm) list"

consts subst :: "string \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm"

inductive Typing :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool"
  ("_ \<turnstile> _ : _" [60, 0, 0] 60)
where
  T_prop: "\<Gamma> \<turnstile> PropSort : Universe 0""#,
        );
        for signature in rules.iter().filter_map(schema) {
            out.push_str(&format!(
                "\n| {}: \"{}\"",
                signature.constructor, signature.proposition
            ));
        }
        out
    }

    fn render_rule_lemma(&self, rule: &RuleSpec) -> String {
        let note = format!("(* {} *)", rule.export_note());
        let Some(schema) = schema(rule) else {
            if rule.status.is_supported() {
                return format!("{note}\n(* Outside this structural typing relation. *)");
            }
            return note;
        };
        format!(
            "{note}\nlemma {}: \"{}\"\n  by (fact {})",
            rule.lemma_name, schema.proposition, schema.constructor
        )
    }

    fn render_main_theorem(&self, rules: &[RuleSpec]) -> String {
        let mut out = String::from("lemmas structural_schema_lemmas =");
        for rule in rules {
            out.push_str(&format!("\n  {}", rule.lemma_name));
        }
        out
    }

    fn render_postscript(&self) -> String {
        "end".into()
    }
}

#[cfg(test)]
pub(crate) fn iou_axioms_isabelle() -> &'static str {
    use std::sync::OnceLock;
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE
        .get_or_init(super::render_iou_axioms_isabelle)
        .as_str()
}

fn schema(rule: &RuleSpec) -> Option<TypingSchema> {
    if !rule.status.is_supported() {
        return None;
    }
    let (constructor, proposition) = match rule.rule_name.as_str() {
        "K_Var" => ("T_var", r#"map_of \<Gamma> x = Some T \<Longrightarrow> \<Gamma> \<turnstile> Var x : T"#),
        "K_Univ" => ("T_univ", r#"i < 2^32 - 1 \<Longrightarrow> \<Gamma> \<turnstile> Universe i : Universe (Suc i)"#),
        "K_Pi_Form" => ("T_pi", r#"\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> B : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Pi x A B : Universe i"#),
        "K_Lam_Intro" => ("T_lam", r#"\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> b : B\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Lam x A b : Pi x A B"#),
        "K_App_Elim" => ("T_app", r#"\<lbrakk>\<Gamma> \<turnstile> f : Pi x A B; \<Gamma> \<turnstile> a : A\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> App f a : subst x a B"#),
        "K_Sigma_Form" => ("T_sigma", r#"\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> B : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Sigma x A B : Universe i"#),
        "K_Pair_Intro" => ("T_pair", r#"\<lbrakk>\<Gamma> \<turnstile> Sigma x A B : Universe i; \<Gamma> \<turnstile> a : A; \<Gamma> \<turnstile> b : subst x a B\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Pair a b : Sigma x A B"#),
        "K_Fst_Elim" => ("T_fst", r#"\<Gamma> \<turnstile> p : Sigma x A B \<Longrightarrow> \<Gamma> \<turnstile> Fst p : A"#),
        "K_Snd_Elim" => ("T_snd", r#"\<Gamma> \<turnstile> p : Sigma x A B \<Longrightarrow> \<Gamma> \<turnstile> Snd p : subst x (Fst p) B"#),
        _ => return None,
    };
    Some(TypingSchema {
        constructor,
        proposition,
    })
}
