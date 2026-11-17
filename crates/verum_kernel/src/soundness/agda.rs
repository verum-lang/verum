//! A safe Cubical Agda syntax and rule inventory. The inventory contains no
//! typing judgment, postulated witness, or aggregate soundness theorem.

use super::{RuleSpec, SchemaEvidence, SoundnessBackend};

/// Cubical Agda emitter — implements [`SoundnessBackend`].
pub struct AgdaBackend;

impl AgdaBackend {
    /// Construct a fresh backend.
    pub fn new() -> Self {
        Self
    }
}

impl Default for AgdaBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundnessBackend for AgdaBackend {
    fn id(&self) -> &'static str {
        "agda"
    }

    fn schema_evidence(&self, rule: &RuleSpec) -> SchemaEvidence {
        if rule.status.is_supported() {
            SchemaEvidence::CatalogueOnly
        } else {
            SchemaEvidence::Unavailable
        }
    }

    fn output_filename(&self) -> &'static str {
        "KernelSoundness.agda"
    }

    fn render_preamble(&self) -> String {
        r#"{-# OPTIONS --cubical --safe #-}
-- KernelSoundness.agda. Syntax and rule inventory only.
-- Agda checks the data declarations. Executable typing and soundness are unverified.
module KernelSoundness where

open import Agda.Builtin.Nat using (Nat)
open import Agda.Builtin.String using (String)
open import Agda.Builtin.List using (List)
open import Agda.Builtin.Sigma using (Σ)"#
            .into()
    }

    fn render_core_term_inductive(&self) -> String {
        String::from(
            "------------------------------------------------------------\n\
             -- `CoreTerm` — kernel calculus syntax (mirror of\n\
             -- `verum_kernel::CoreTerm` at term.rs).\n\
             ------------------------------------------------------------\n\
             data CoreTerm : Set where\n  \
               Var          : String → CoreTerm\n  \
               Universe     : Nat → CoreTerm\n  \
               PropSort     : CoreTerm\n  \
               Pi           : String → CoreTerm → CoreTerm → CoreTerm\n  \
               Lam          : String → CoreTerm → CoreTerm → CoreTerm\n  \
               App          : CoreTerm → CoreTerm → CoreTerm\n  \
               Sigma        : String → CoreTerm → CoreTerm → CoreTerm\n  \
               Pair         : CoreTerm → CoreTerm → CoreTerm\n  \
               Fst          : CoreTerm → CoreTerm\n  \
               Snd          : CoreTerm → CoreTerm\n  \
               PathTy       : CoreTerm → CoreTerm → CoreTerm → CoreTerm\n  \
               PathOver     : CoreTerm → CoreTerm → CoreTerm → CoreTerm → CoreTerm\n  \
               Refl         : CoreTerm → CoreTerm\n  \
               HComp        : CoreTerm → CoreTerm → CoreTerm → CoreTerm\n  \
               Transport    : CoreTerm → CoreTerm → CoreTerm\n  \
               Glue         : CoreTerm → CoreTerm → CoreTerm → CoreTerm → CoreTerm\n  \
               Refine       : CoreTerm → String → CoreTerm → CoreTerm\n  \
               Quotient     : CoreTerm → CoreTerm → CoreTerm\n  \
               QuotIntro    : CoreTerm → CoreTerm → CoreTerm → CoreTerm\n  \
               QuotElim     : CoreTerm → CoreTerm → CoreTerm → CoreTerm → CoreTerm\n  \
               InductiveT   : String → List CoreTerm → CoreTerm\n  \
               Elim         : CoreTerm → CoreTerm → List CoreTerm → CoreTerm\n  \
               SmtProof     : String → CoreTerm\n  \
               AxiomT       : String → CoreTerm → String → CoreTerm\n  \
               EpsilonOf    : CoreTerm → CoreTerm\n  \
               AlphaOf      : CoreTerm → CoreTerm\n  \
               ModalBox     : CoreTerm → CoreTerm\n  \
               ModalDiamond : CoreTerm → CoreTerm\n  \
               ModalBigAnd  : List CoreTerm → CoreTerm\n  \
               Shape        : CoreTerm → CoreTerm\n  \
               Flat         : CoreTerm → CoreTerm\n  \
               Sharp        : CoreTerm → CoreTerm",
        )
    }

    fn render_core_type_inductive(&self) -> String {
        "Ctx : Set\nCtx = List (Σ String (λ _ → CoreTerm))".into()
    }

    fn render_kernel_rule_inductive(&self, rules: &[RuleSpec]) -> String {
        let mut out = String::from(
            "------------------------------------------------------------\n\
             -- `KernelRule` — the supported kernel rule taxonomy\n\
             -- (mirror of `verum_kernel::proof_tree::KernelRule`).\n\
             ------------------------------------------------------------\n\
             data KernelRule : Set where\n  ",
        );
        let names: Vec<String> = rules
            .iter()
            .map(|r| r.rule_name.replace('_', "-"))
            .collect();
        for (i, n) in names.iter().enumerate() {
            if i > 0 {
                out.push_str("\n  ");
            }
            out.push_str(&format!("{} : KernelRule", n));
        }

        out
    }

    fn render_rule_lemma(&self, rule: &RuleSpec) -> String {
        format!("-- {}", rule.export_note())
    }

    fn render_main_theorem(&self, _rules: &[RuleSpec]) -> String {
        String::new()
    }

    fn render_postscript(&self) -> String {
        "-- End of syntax and rule inventory.".into()
    }
}
