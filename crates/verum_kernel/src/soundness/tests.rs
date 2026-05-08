//! Tests for the kernel-soundness cross-export pipeline.
//!

//! Pin coverage:
//!  - The canonical rule list has exactly 35 entries.
//!  - The 4 structurally-proved lemmas (K-Var, K-Univ, K-FwAx, K-Pos)
//!  are marked `Proved`; everything else is `Admitted` with a
//!  non-empty reason.
//!  - Coq + Lean backends emit non-empty output for every section
//!  and every rule; Admitted lemmas carry their reason in a
//!  comment alongside the `Admitted.` / `sorry`.
//!  - The drift check fires when the rule list differs from
//!  `EXPECTED_KERNEL_RULE_COUNT`.

use super::coq::CoqBackend;
use super::lean::LeanBackend;
use super::{
    EXPECTED_KERNEL_RULE_COUNT, LemmaStatus, RuleSpec, SoundnessBackend, SoundnessExporter,
    canonical_rules,
};

#[test]
fn canonical_rules_has_expected_count() {
    let rules = canonical_rules();
    assert_eq!(
        rules.len(),
        EXPECTED_KERNEL_RULE_COUNT,
        "canonical_rules() should produce exactly {} entries",
        EXPECTED_KERNEL_RULE_COUNT,
    );
}

#[test]
fn proved_lemma_set_full_post_discharge() {
    // Pin: 4 placeholder + 9 IOU discharges + 10 status-fixes
    // (rules already structural in the export since FV-9; mod.rs
    // status now matches) = 23 proved.
    //
    // The 10 status-fixes don't change the IOU count (none had an
    // axiom in the export to begin with) — they close a drift
    // between mod.rs LemmaStatus and the export shape.
    let rules = canonical_rules();
    let proved: Vec<&str> = rules
        .iter()
        .filter(|r| r.status.is_proved())
        .map(|r| r.rule_name.as_str())
        .collect();

    assert_eq!(
        proved.len(),
        23,
        "expected 23 structurally-proved lemmas, got {}: {:?}",
        proved.len(),
        proved,
    );

    for needed in [
        // 4 placeholder structural rules
        "K_Var", "K_Univ", "K_FwAx", "K_Pos",
        // 9 IOU discharges (this session)
        "K_Quot_Elim", "K_Elim", "K_Universe_Ascent",
        "K_Refine", "K_Refine_Omega", "K_Inductive",
        "K_Epsilon_Of", "K_Alpha_Of", "K_Modal_Big_And",
        // 5 modal/cohesive status-fixes (export was structural since FV-9)
        "K_Modal_Box", "K_Modal_Diamond",
        "K_Shape", "K_Flat", "K_Sharp",
        // 5 more status-fixes (same drift pattern)
        "K_Path_Ty_Form", "K_Refl_Intro",
        "K_Refine_Erase", "K_Quot_Form", "K_Quot_Intro",
    ] {
        assert!(
            proved.contains(&needed),
            "{} must be proved",
            needed,
        );
    }
}

#[test]
fn every_admitted_lemma_has_non_empty_reason() {
    let rules = canonical_rules();
    for r in &rules {
        if let LemmaStatus::Admitted { reason } = &r.status {
            assert!(
                !reason.trim().is_empty(),
                "rule {} is admitted but reason is empty — admits must be \
                 audit-able IOUs, not silent placeholders",
                r.rule_name,
            );
        }
    }
}

#[test]
fn every_discharged_lemma_has_non_empty_citation() {
    let rules = canonical_rules();
    for r in &rules {
        if let LemmaStatus::DischargedByFramework {
            lemma_path,
            framework,
            citation,
        } = &r.status
        {
            assert!(
                !lemma_path.trim().is_empty(),
                "rule {} discharged-by-framework but lemma_path is empty",
                r.rule_name,
            );
            assert!(
                !framework.trim().is_empty(),
                "rule {} discharged-by-framework but framework is empty",
                r.rule_name,
            );
            assert!(
                !citation.trim().is_empty(),
                "rule {} discharged-by-framework but citation is empty — \
                 the audit gate's trust-extension report needs the upstream path",
                r.rule_name,
            );
        }
    }
}

#[test]
fn discharged_count_reflects_phase_1a() {
    let rules = canonical_rules();
    let exporter = SoundnessExporter::new();
    let _ = rules.len();
    // Post-#155 Phase-1A: at least 7 rules discharged by framework
    // citation (K_Pi_Form, K_Lam_Intro, K_App_Elim, K_Sigma_Form,
    // K_Pair_Intro, K_Fst_Elim, K_Snd_Elim). This count is a floor;
    // future Phase-1A advances will increase it.
    assert!(
        exporter.discharged_by_framework_count() >= 7,
        "expected at least 7 framework-discharged rules post-#155 Phase-1A, got {}",
        exporter.discharged_by_framework_count(),
    );
}

#[test]
fn rule_names_are_unique() {
    let rules = canonical_rules();
    let mut seen = std::collections::HashSet::new();
    for r in &rules {
        assert!(
            seen.insert(r.rule_name.clone()),
            "duplicate rule name: {}",
            r.rule_name,
        );
    }
}

#[test]
fn lemma_names_match_rule_names_with_sound_suffix() {
    for r in &canonical_rules() {
        assert_eq!(
            r.lemma_name,
            format!("{}_sound", r.rule_name),
            "lemma_name should be `{}_sound`",
            r.rule_name,
        );
    }
}

#[test]
fn soundness_backend_id_resolves_to_canonical_foreign_system() {
    use crate::foreign_system::ForeignSystem;
    let coq = CoqBackend::new();
    let lean = LeanBackend::new();
    assert_eq!(coq.foreign_system(), Some(ForeignSystem::Coq));
    assert_eq!(lean.foreign_system(), Some(ForeignSystem::Lean4));
}

#[test]
fn coq_backend_emits_full_file() {
    let exporter = SoundnessExporter::new();
    let coq = CoqBackend::new();
    let output = exporter.emit(&coq);

    // Preamble
    assert!(output.contains("kernel_soundness.v"));
    assert!(output.contains("GENERATED by"));
    // Inductives
    assert!(output.contains("Inductive CoreTerm"));
    assert!(output.contains("Inductive CoreType"));
    assert!(output.contains("Inductive KernelRule"));
    // All 35 rules appear as constructors of KernelRule and as lemmas
    for r in exporter.rules() {
        assert!(
            output.contains(&r.rule_name),
            "Coq output missing rule {}",
            r.rule_name,
        );
        assert!(
            output.contains(&r.lemma_name),
            "Coq output missing lemma {}",
            r.lemma_name,
        );
    }
    // Top theorem
    assert!(output.contains("Theorem kernel_soundness"));
    // Postscript
    assert!(output.contains("End of kernel_soundness.v"));
    // Filename matches
    assert_eq!(coq.output_filename(), "kernel_soundness.v");
}

#[test]
fn coq_backend_renders_admitted_with_reason_comment() {
    let exporter = SoundnessExporter::new();
    let coq = CoqBackend::new();
    let output = exporter.emit(&coq);

    // Pick a discharged lemma and confirm `Admitted.` plus the
    // citation appear. K_Pi_Form is now discharged-by-framework
    // citing the substitution-lemma in mathlib4.
    assert!(output.contains("Admitted."));
    assert!(
        output.contains("substitution-lemma") || output.contains("Substitution"),
        "Coq emission must carry the K_Pi_Form discharge citation",
    );
}

#[test]
fn coq_backend_renders_proved_lemmas_with_qed() {
    let exporter = SoundnessExporter::new();
    let coq = CoqBackend::new();
    let output = exporter.emit(&coq);

    // Proved lemmas end in Qed., not Admitted.
    let var_lemma_pos = output
        .find("Lemma K_Var_sound")
        .expect("K_Var_sound lemma must be present");
    let next_qed = output[var_lemma_pos..]
        .find("Qed.")
        .expect("K_Var_sound's proof must end in Qed.");
    let next_admitted = output[var_lemma_pos..].find("Admitted.");
    if let Some(adm) = next_admitted {
        assert!(
            next_qed < adm,
            "K_Var_sound must end in Qed. before any later Admitted.",
        );
    }
}

#[test]
fn lean_backend_emits_full_file() {
    let exporter = SoundnessExporter::new();
    let lean = LeanBackend::new();
    let output = exporter.emit(&lean);

    assert!(output.contains("KernelSoundness.lean"));
    assert!(output.contains("namespace KernelSoundness"));
    assert!(output.contains("inductive CoreTerm"));
    assert!(output.contains("inductive CoreType"));
    assert!(output.contains("inductive KernelRule"));
    for r in exporter.rules() {
        assert!(
            output.contains(&r.rule_name),
            "Lean output missing rule {}",
            r.rule_name,
        );
        assert!(
            output.contains(&r.lemma_name),
            "Lean output missing lemma {}",
            r.lemma_name,
        );
    }
    assert!(output.contains("theorem kernel_soundness"));
    assert!(output.contains("end KernelSoundness"));
    assert_eq!(lean.output_filename(), "KernelSoundness.lean");
}

#[test]
fn lean_backend_renders_admitted_with_reason_comment() {
    let exporter = SoundnessExporter::new();
    let lean = LeanBackend::new();
    let output = exporter.emit(&lean);

    assert!(
        output.contains("sorry"),
        "Lean emission must use `sorry` for admitted/discharged lemmas",
    );
    assert!(
        output.contains("substitution-lemma") || output.contains("Substitution"),
        "Lean emission must carry the K_Pi_Form discharge citation",
    );
}

#[test]
fn drift_check_passes_for_canonical_rules() {
    let exporter = SoundnessExporter::new();
    assert!(
        exporter.drift_check().is_ok(),
        "canonical rule list must pass the drift check",
    );
}

#[test]
fn drift_check_rejects_short_list() {
    let short_rules: Vec<RuleSpec> = canonical_rules().into_iter().take(10).collect();
    let exporter = SoundnessExporter::with_rules(short_rules);
    let err = exporter
        .drift_check()
        .expect_err("short list must fail drift check");
    assert!(err.contains("10 rules"));
    assert!(err.contains(&format!("expected {}", EXPECTED_KERNEL_RULE_COUNT)));
}

#[test]
fn proved_count_plus_admitted_count_matches_total() {
    let exporter = SoundnessExporter::new();
    let proved = exporter.proved_count();
    let admitted = exporter.admitted_count();
    let discharged = exporter.discharged_by_framework_count();
    assert_eq!(
        proved + admitted + discharged,
        EXPECTED_KERNEL_RULE_COUNT,
        "every rule must be either proved, admitted, or discharged-by-framework",
    );
    // 4 placeholder + 9 IOU discharges + 10 status-fixes (rules
    // already structural in export since FV-9; mod.rs status now
    // matches) = 23 proved.
    assert_eq!(proved, 23, "expected 23 proved lemmas");
    assert!(
        discharged >= 7,
        "expected at least 7 framework-discharged lemmas post-#155 Phase-1A, got {}",
        discharged,
    );
    assert_eq!(
        proved + admitted + discharged,
        EXPECTED_KERNEL_RULE_COUNT,
        "proved + admitted + discharged must total all rules",
    );
}

#[test]
fn admitted_iou_list_enumerates_every_admit() {
    let exporter = SoundnessExporter::new();
    let ious = exporter.admitted_iou_list();
    // Includes both `Admitted` (open IOU) and `DischargedByFramework`
    // (closed IOU with citation) — both contribute to the trust-
    // extension surface that the audit gate enumerates.
    assert_eq!(
        ious.len(),
        exporter.admitted_count() + exporter.discharged_by_framework_count(),
        "the IOU list must enumerate every admitted + discharged-by-framework lemma",
    );
    for (rule_name, reason) in ious {
        assert!(
            !reason.is_empty(),
            "IOU for {} has empty reason/citation",
            rule_name
        );
    }
}

#[test]
fn coq_main_theorem_dispatches_to_each_lemma() {
    let exporter = SoundnessExporter::new();
    let coq = CoqBackend::new();
    let output = exporter.emit(&coq);

    // The main theorem case-analyses on KernelRule and apply each
    // K_<Name>_sound lemma per branch.
    for r in exporter.rules() {
        let dispatch = format!("apply ({}", r.lemma_name);
        assert!(
            output.contains(&dispatch),
            "Coq main theorem must dispatch to {}",
            r.lemma_name,
        );
    }
}

#[test]
fn lean_main_theorem_dispatches_to_each_lemma() {
    let exporter = SoundnessExporter::new();
    let lean = LeanBackend::new();
    let output = exporter.emit(&lean);

    for r in exporter.rules() {
        // Lean dispatch shape: `case <RuleName> => exact <LemmaName> ...`
        let dispatch = format!("case {} => exact {}", r.rule_name, r.lemma_name);
        assert!(
            output.contains(&dispatch),
            "Lean main theorem must dispatch to {}",
            r.lemma_name,
        );
    }
}

#[test]
fn rule_categories_partition_the_corpus() {
    let exporter = SoundnessExporter::new();
    let mut counts = std::collections::HashMap::<&'static str, usize>::new();
    for r in exporter.rules() {
        *counts.entry(r.category.tag()).or_insert(0) += 1;
    }
    // The category counts must match the documented architecture in
    // model.vr / judgment.vr: Structural 9, Cubical 6, Refinement 4,
    // Quotient 3, Inductive 3, SmtAxiom 2, Diakrisis 11.
    assert_eq!(counts.get("Structural"), Some(&9));
    assert_eq!(counts.get("Cubical"), Some(&6));
    assert_eq!(counts.get("Refinement"), Some(&4));
    assert_eq!(counts.get("Quotient"), Some(&3));
    assert_eq!(counts.get("Inductive"), Some(&3));
    assert_eq!(counts.get("SmtAxiom"), Some(&2));
    assert_eq!(counts.get("Diakrisis"), Some(&11));
    let total: usize = counts.values().sum();
    assert_eq!(total, EXPECTED_KERNEL_RULE_COUNT);
}

// =============================================================================
// Drift-guard tests (PR-1): mod.rs LemmaStatus ↔ export IOU axiom presence
// =============================================================================

#[test]
fn drift_check_passes_at_baseline() {
    // Pin: in the as-shipped state, drift_check passes — every
    // mod.rs `Admitted` rule has a corresponding `<Rule>_iou`
    // axiom in the export, and every `Proved` /
    // `DischargedByFramework` rule does NOT.
    let exporter = SoundnessExporter::new();
    let result = exporter.drift_check();
    assert!(
        result.is_ok(),
        "drift_check must pass at baseline; got: {:?}",
        result,
    );
}

#[test]
fn drift_check_catches_admitted_without_iou_axiom() {
    // Pin: a rule that's Admitted in mod.rs but has no IOU axiom
    // in the export (PR-5g/PR-5h drift pattern) is caught by
    // drift_check.
    use crate::soundness::{
        LemmaStatus, RuleCategory, RuleSpec, SoundnessExporter,
    };
    let mut rules: Vec<RuleSpec> = canonical_rules();
    // K_Quot_Elim has no IOU axiom (discharged in PR-5).  Flipping
    // it back to Admitted should trigger a drift error.
    for r in rules.iter_mut() {
        if r.rule_name == "K_Quot_Elim" {
            r.status = LemmaStatus::Admitted {
                reason: "synthetic drift to test the guard".to_string(),
            };
        }
    }
    let _ = RuleCategory::Quotient; // imports
    let exporter = SoundnessExporter::with_rules(rules);
    let err = exporter
        .drift_check()
        .expect_err("drift_check should reject Admitted-without-axiom");
    assert!(
        err.contains("K_Quot_Elim"),
        "drift error should name K_Quot_Elim; got: {}",
        err,
    );
    assert!(
        err.contains("no") && err.contains("axiom"),
        "drift error should name the missing axiom; got: {}",
        err,
    );
}

#[test]
fn drift_check_catches_proved_with_orphan_iou_axiom() {
    // Pin: a rule that's Proved in mod.rs but still has an IOU
    // axiom in the export (orphan-axiom drift) is caught.
    use crate::soundness::{
        LemmaStatus, RuleSpec, SoundnessExporter,
    };
    let mut rules: Vec<RuleSpec> = canonical_rules();
    // K_Smt has an IOU axiom and is currently Admitted.  Flipping
    // it to Proved without removing the axiom should trigger a
    // drift error.
    for r in rules.iter_mut() {
        if r.rule_name == "K_Smt" {
            r.status = LemmaStatus::Proved {
                coq_tactics: "exact T_smt.".to_string(),
                lean_tactics: "  exact @Typing.t_smt _ _ _".to_string(),
            };
        }
    }
    let exporter = SoundnessExporter::with_rules(rules);
    let err = exporter
        .drift_check()
        .expect_err("drift_check should reject Proved-with-orphan-axiom");
    assert!(
        err.contains("K_Smt"),
        "drift error should name K_Smt; got: {}",
        err,
    );
    assert!(
        err.contains("orphan"),
        "drift error should call out the orphan axiom; got: {}",
        err,
    );
}

#[test]
fn iou_axiom_rule_names_count_matches_admitted_count() {
    // Pin: the IOU-axiom-source-of-truth list is the same length
    // as the count of Admitted rules in mod.rs.  This is the
    // partner pin to drift_check: it catches a state where the
    // two sides have correct membership but a mis-counted total.
    use crate::soundness::iou_axiom_rule_names;
    let exporter = SoundnessExporter::new();
    let admitted_count = exporter.admitted_count();
    let iou_count = iou_axiom_rule_names().len();
    assert_eq!(
        admitted_count, iou_count,
        "Admitted-count ({}) and IOU-axiom-count ({}) must match — \
         every Admitted rule contributes one IOU axiom",
        admitted_count, iou_count,
    );
}

#[test]
fn iou_axiom_rule_names_match_admitted_rule_names() {
    // Pin: the IOU-axiom rule names match the rule names of every
    // Admitted lemma — set equality.
    use crate::soundness::iou_axiom_rule_names;
    let exporter = SoundnessExporter::new();
    let admitted_names: std::collections::BTreeSet<String> = exporter
        .rules()
        .iter()
        .filter(|r| matches!(r.status, LemmaStatus::Admitted { .. }))
        .map(|r| r.rule_name.clone())
        .collect();
    let iou_names: std::collections::BTreeSet<String> = iou_axiom_rule_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        admitted_names, iou_names,
        "Admitted rule set must equal IOU-axiom rule set",
    );
}
