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
use super::isabelle::IsabelleBackend;
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
    // Pin: 4 placeholder + 15 IOU discharges + 10 status-fixes
    // (rules already structural in the export; mod.rs status now
    // matches) = 29 proved.  The FV-17 final batch closed the
    // last open structurally-dischargeable IOU (K_Smt) plus moved
    // K_Eps_Mu and K_Round_Trip into the DischargedByFramework
    // bucket — leaving `iou_axiom_specs()` empty.
    let rules = canonical_rules();
    let proved: Vec<&str> = rules
        .iter()
        .filter(|r| r.status.is_proved())
        .map(|r| r.rule_name.as_str())
        .collect();

    assert_eq!(
        proved.len(),
        29,
        "expected 29 structurally-proved lemmas, got {}: {:?}",
        proved.len(),
        proved,
    );

    for needed in [
        // 4 placeholder structural rules
        "K_Var", "K_Univ", "K_FwAx", "K_Pos",
        // 15 IOU discharges (4 Cubical + 4 Refinement + 2 Inductive
        // + 1 Universe_Ascent + 3 Diakrisis structural-fragment + 1 SmtAxiom).
        "K_Quot_Elim", "K_Elim", "K_Universe_Ascent",
        "K_Refine", "K_Refine_Omega", "K_Refine_Intro", "K_Inductive",
        "K_Path_Over_Form", "K_HComp", "K_Transp", "K_Glue",
        "K_Epsilon_Of", "K_Alpha_Of", "K_Modal_Big_And",
        "K_Smt",
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
fn trust_extension_report_partitions_corpus() {
    // Pin (FV-18): the trust-extension report's aggregate counts
    // partition the 38-rule corpus exactly across {Proved,
    // Admitted, DischargedByFramework}.  Post-FV-17 the IOU
    // registry is empty, so the open-axiom count is `0`.
    let exporter = SoundnessExporter::new();
    let report = exporter.render_trust_extension_report();

    assert_eq!(
        report.total_rules,
        EXPECTED_KERNEL_RULE_COUNT,
        "trust-extension report must enumerate every kernel rule",
    );
    assert_eq!(
        report.proved_count + report.admitted_count + report.discharged_by_framework_count,
        report.total_rules,
        "Proved + Admitted + DischargedByFramework must partition the corpus",
    );
    assert_eq!(report.entries.len(), report.total_rules);

    // Post-FV-17 architectural endgame: zero open IOUs.
    assert_eq!(
        report.open_iou_axioms,
        0,
        "trust-extension report must witness empty IOU registry post-FV-17",
    );
    assert_eq!(
        report.open_iou_axioms,
        crate::soundness::iou_axiom_specs().len(),
        "report's open_iou_axioms must equal `iou_axiom_specs().len()`",
    );

    // Per-rule status kind shape.
    for entry in &report.entries {
        match entry.status_kind.as_str() {
            "proved" => {
                assert!(
                    entry.framework_lemma_path.is_none() && entry.framework_name.is_none(),
                    "Proved rule {} must have no framework citation; got: {:?}",
                    entry.rule_name,
                    entry,
                );
            }
            "discharged-by-framework" => {
                assert!(
                    entry.framework_lemma_path.is_some() && entry.framework_name.is_some(),
                    "DischargedByFramework rule {} must carry citation triple; got: {:?}",
                    entry.rule_name,
                    entry,
                );
            }
            "admitted" => {
                assert!(
                    entry.trust_note.is_some(),
                    "Admitted rule {} must carry a reason; got: {:?}",
                    entry.rule_name,
                    entry,
                );
            }
            other => panic!("unexpected status_kind {:?} on rule {}", other, entry.rule_name),
        }
    }
}

#[test]
fn trust_extension_report_serialises_to_json() {
    // Pin (FV-18): the report's structure is JSON-serialisable
    // (Serde `Serialize` derive) and round-trips through JSON
    // back into the same logical value.  This is the audit-gate
    // contract for `--trust-extension-report --format json`.
    let exporter = SoundnessExporter::new();
    let report = exporter.render_trust_extension_report();
    let json = serde_json::to_string(&report)
        .expect("trust-extension report must serialise to JSON");
    let round_trip: crate::soundness::TrustExtensionReport =
        serde_json::from_str(&json).expect("JSON round-trip must succeed");
    // Structural equality on the load-bearing aggregate fields —
    // entry-by-entry equality would over-pin.
    assert_eq!(round_trip.total_rules, report.total_rules);
    assert_eq!(round_trip.proved_count, report.proved_count);
    assert_eq!(
        round_trip.discharged_by_framework_count,
        report.discharged_by_framework_count,
    );
    assert_eq!(round_trip.admitted_count, report.admitted_count);
    assert_eq!(round_trip.open_iou_axioms, report.open_iou_axioms);
    assert_eq!(round_trip.entries.len(), report.entries.len());
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
    // 4 placeholder + 15 IOU discharges + 10 status-fixes (rules
    // already structural in export; mod.rs status now matches) =
    // 29 proved (post-FV-17 final batch — `iou_axiom_specs()` empty).
    assert_eq!(proved, 29, "expected 29 proved lemmas");
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
fn isabelle_backend_emits_full_file() {
    let exporter = SoundnessExporter::new();
    let isa = IsabelleBackend::new();
    let output = exporter.emit(&isa);

    assert!(output.contains("KernelSoundness.thy"));
    assert!(output.contains("theory KernelSoundness"));
    assert!(output.contains("datatype CoreTerm"));
    assert!(output.contains("datatype CoreType"));
    assert!(output.contains("datatype KernelRule"));
    for r in exporter.rules() {
        assert!(
            output.contains(&r.rule_name),
            "Isabelle output missing rule {}",
            r.rule_name,
        );
        assert!(
            output.contains(&r.lemma_name),
            "Isabelle output missing lemma {}",
            r.lemma_name,
        );
    }
    // Isabelle main theorem is the `lemmas kernel_full_soundness =`
    // bundle (architectural asymmetry with Lean / Coq — see
    // `render_main_theorem` in soundness/isabelle.rs for rationale).
    assert!(
        output.contains("lemmas kernel_full_soundness ="),
        "Isabelle output must declare the kernel_full_soundness bundle",
    );
    assert!(
        !output.contains("definition Soundness"),
        "Isabelle output must NOT emit the case-of `definition Soundness` \
         shape — its 38-branch elaboration is a non-converging unification \
         problem.  See render_main_theorem comment.",
    );
    assert_eq!(isa.output_filename(), "KernelSoundness.thy");
}

#[test]
fn isabelle_backend_renders_admitted_via_axiomatization() {
    let exporter = SoundnessExporter::new();
    let isa = IsabelleBackend::new();
    let output = exporter.emit(&isa);

    // `Admitted` / `DischargedByFramework` rules emit a per-rule
    // `axiomatization where K_X_sound: "..."` block carrying the
    // same propositional statement.  This keeps the lemma name
    // resolvable by `lemmas kernel_full_soundness =` without
    // requiring `quick_and_dirty` mode (which `sorry` would
    // demand).
    assert!(
        !output.contains("sorry"),
        "Isabelle emission must NOT use `sorry` (requires \
         quick_and_dirty mode); use axiomatization instead",
    );
    // Find at least one DischargedByFramework rule and verify it
    // emits as an axiomatization, not a `lemma ... sorry`.
    let discharged_rule = exporter
        .rules()
        .iter()
        .find(|r| matches!(r.status, super::LemmaStatus::DischargedByFramework { .. }))
        .expect("registry must have at least one DischargedByFramework rule");
    let expected = format!(
        "axiomatization where {}",
        discharged_rule.lemma_name,
    );
    assert!(
        output.contains(&expected),
        "DischargedByFramework rule {} must emit as `axiomatization where {} : ...`",
        discharged_rule.rule_name,
        discharged_rule.lemma_name,
    );
    assert!(
        output.contains("substitution-lemma") || output.contains("Substitution"),
        "Isabelle emission must carry the K_Pi_Form discharge citation",
    );
}

#[test]
fn isabelle_main_theorem_lists_every_per_rule_lemma() {
    // Pin: the `lemmas kernel_full_soundness = ...` bundle must
    // enumerate every per-rule lemma in the registry.  Auditors
    // invoke `print_facts kernel_full_soundness` to enumerate
    // every soundness fact at once — drift between the registry
    // and the bundle would silently drop rules.
    let exporter = SoundnessExporter::new();
    let isa = IsabelleBackend::new();
    let output = exporter.emit(&isa);

    let bundle_marker = "lemmas kernel_full_soundness =";
    let bundle_idx = output
        .find(bundle_marker)
        .expect("Isabelle output must declare the kernel_full_soundness bundle");
    let bundle_tail = &output[bundle_idx..];

    for r in exporter.rules() {
        assert!(
            bundle_tail.contains(&r.lemma_name),
            "kernel_full_soundness bundle must list every per-rule lemma \
             — missing {}",
            r.lemma_name,
        );
    }
}

#[test]
fn isabelle_emit_does_not_use_case_of_aggregate() {
    // Pin: the Isabelle backend MUST NOT regress to the case-of
    // `definition Soundness :: KernelRule ⇒ bool` shape — that
    // form is a non-converging unification problem at universe-
    // polymorphic free-variable density.  See `render_main_theorem`
    // in soundness/isabelle.rs for the architectural rationale.
    let exporter = SoundnessExporter::new();
    let isa = IsabelleBackend::new();
    let output = exporter.emit(&isa);

    assert!(
        !output.contains("case rule of"),
        "Isabelle emission must NOT contain the `case rule of K_X => …` \
         aggregate — it is the bottleneck this backend was refactored \
         away from.  See render_main_theorem doc comment.",
    );
    assert!(
        !output.contains("definition Soundness"),
        "Isabelle emission must NOT contain `definition Soundness` — \
         the per-rule lemmas are the real propositional content.",
    );
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
fn drift_check_catches_admitted_with_missing_iou_axiom() {
    // Pin: a rule that's Admitted in mod.rs but has NO IOU axiom in
    // the export (missing-axiom drift) is caught.  Post-FV-17 the
    // IOU registry is empty by design, so the inverse drift class
    // (orphan axiom) is structurally unreachable; this regression
    // guard remains exercisable because Admitted-without-axiom
    // can still happen if a future change forgets to flip the
    // status when removing an axiom from the registry.
    use crate::soundness::{
        LemmaStatus, RuleSpec, SoundnessExporter,
    };
    let mut rules: Vec<RuleSpec> = canonical_rules();
    // Pick K_Smt as the synthetic Admitted target: it's currently
    // Proved, so flipping it to Admitted (while the registry stays
    // empty) creates the missing-axiom drift the check should
    // catch.
    for r in rules.iter_mut() {
        if r.rule_name == "K_Smt" {
            r.status = LemmaStatus::Admitted {
                reason: "synthetic test fixture".to_string(),
            };
        }
    }
    let exporter = SoundnessExporter::with_rules(rules);
    let err = exporter
        .drift_check()
        .expect_err("drift_check should reject Admitted-with-missing-axiom");
    assert!(
        err.contains("K_Smt"),
        "drift error should name K_Smt; got: {}",
        err,
    );
    assert!(
        err.contains("status drift") || err.contains("no") || err.contains("missing"),
        "drift error should call out the missing axiom; got: {}",
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

// =============================================================================
// Cross-foundation IOU axiom set consistency (PR-1b)
// =============================================================================
//
// Drift surface the aggregate-IOU check doesn't catch on its
// own: each of Lean / Coq / Isabelle renders its own IOU axiom
// block via `iou_axioms_<foundation>()`.  All three pull from
// the same `iou_axiom_specs()` registry, but that wiring is
// load-bearing — if a renderer ever skips a spec or emits the
// wrong name, the auditor reading one foundation's export sees a
// different axiom set than the other two.  These tests pin
// set-equality between `iou_axiom_rule_names()` (single source
// of truth) and the actual axiom names extracted from each
// foundation's rendered block.

/// Extract `K_<Name>_iou` rule names from a per-foundation
/// IOU-axiom string constant.  Pattern-matches on the `_iou`
/// suffix and walks back to the preceding `K_` prefix.  Robust
/// to per-foundation syntax (`axiom`/`Axiom`/`<name> ::`) since
/// it doesn't anchor on the leading keyword.
fn extract_iou_rule_names_from_constant(constant: &str) -> std::collections::BTreeSet<String> {
    let mut result = std::collections::BTreeSet::new();
    // Walk byte by byte looking for the "_iou" suffix.  When found,
    // walk backwards over identifier chars (letters, digits, `_`)
    // collecting the name.  Then verify it starts with `K_` —
    // skip false matches.
    let bytes = constant.as_bytes();
    let needle = b"_iou";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            // Found `_iou`.  Walk backwards over identifier chars.
            let mut start = i;
            while start > 0 {
                let c = bytes[start - 1];
                if c.is_ascii_alphanumeric() || c == b'_' {
                    start -= 1;
                } else {
                    break;
                }
            }
            let name_with_iou: &str = &constant[start..i + needle.len()];
            // Must start with "K_" to be a kernel rule.
            if name_with_iou.starts_with("K_") {
                // Strip "_iou" suffix.
                let rule_name = &name_with_iou[..name_with_iou.len() - needle.len()];
                result.insert(rule_name.to_string());
            }
            i += needle.len();
        } else {
            i += 1;
        }
    }
    result
}

#[test]
fn extractor_finds_axioms_in_lean_constant() {
    // Sanity: the extractor's output cardinality matches the
    // registry's length exactly — the rendered Lean text has one
    // `axiom K_<rule>_iou` line per spec.  Post-FV-17 the registry
    // is empty, so the extractor sees zero axioms; the regression
    // guard catches drift between `iou_axiom_specs()` and the
    // rendered text if either side is altered without the other.
    use crate::soundness::lean::iou_axioms_lean;
    let names = extract_iou_rule_names_from_constant(iou_axioms_lean());
    let expected = crate::soundness::iou_axiom_specs().len();
    assert_eq!(
        names.len(),
        expected,
        "extractor saw {} axioms; iou_axiom_specs() declares {}: {:?}",
        names.len(),
        expected,
        names,
    );
    // Sanity check on the lower-bound assertion (kept for shape
    // backwards compat with prior versions that asserted a
    // hard-coded floor).
    assert!(
        names.len() >= expected,
        "extractor should find at least {} IOU axioms; got {}: {:?}",
        expected,
        names.len(),
        names,
    );
}

#[test]
fn lean_constant_iou_axioms_match_source_of_truth() {
    // Pin: the Lean `iou_axioms_lean()` constant declares exactly the
    // axioms in `iou_axiom_rule_names()`.  Drift here means a
    // discharge updated mod.rs but forgot to remove the Lean axiom
    // (or vice versa).
    use crate::soundness::iou_axiom_rule_names;
    use crate::soundness::lean::iou_axioms_lean;
    let extracted = extract_iou_rule_names_from_constant(iou_axioms_lean());
    let expected: std::collections::BTreeSet<String> = iou_axiom_rule_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        extracted, expected,
        "Lean iou_axioms_lean() constant must match iou_axiom_rule_names() set",
    );
}

#[test]
fn coq_constant_iou_axioms_match_source_of_truth() {
    // Pin: the Coq `iou_axioms_coq()` constant declares exactly the
    // axioms in `iou_axiom_rule_names()`.
    use crate::soundness::coq::iou_axioms_coq;
    use crate::soundness::iou_axiom_rule_names;
    let extracted = extract_iou_rule_names_from_constant(iou_axioms_coq());
    let expected: std::collections::BTreeSet<String> = iou_axiom_rule_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        extracted, expected,
        "Coq iou_axioms_coq() constant must match iou_axiom_rule_names() set",
    );
}

#[test]
fn isabelle_constant_iou_axioms_match_source_of_truth() {
    // Pin: the Isabelle `iou_axioms_isabelle()` constant declares exactly
    // the axioms in `iou_axiom_rule_names()`.
    use crate::soundness::iou_axiom_rule_names;
    use crate::soundness::isabelle::iou_axioms_isabelle;
    let extracted = extract_iou_rule_names_from_constant(iou_axioms_isabelle());
    let expected: std::collections::BTreeSet<String> = iou_axiom_rule_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        extracted, expected,
        "Isabelle iou_axioms_isabelle() constant must match iou_axiom_rule_names() set",
    );
}

#[test]
fn three_foundations_agree_on_iou_axiom_set() {
    // Pin: Lean and Coq and Isabelle all declare the SAME set of
    // IOU axioms.  Direct three-way agreement check (no detour
    // through `iou_axiom_rule_names()`).  If one foundation drifts
    // from the others, this fires immediately — separating
    // "axiom name present" drift from "rule status" drift.
    use crate::soundness::coq::iou_axioms_coq;
    use crate::soundness::isabelle::iou_axioms_isabelle;
    use crate::soundness::lean::iou_axioms_lean;
    let lean_set = extract_iou_rule_names_from_constant(iou_axioms_lean());
    let coq_set = extract_iou_rule_names_from_constant(iou_axioms_coq());
    let isa_set = extract_iou_rule_names_from_constant(iou_axioms_isabelle());
    assert_eq!(lean_set, coq_set, "Lean and Coq IOU axiom sets must agree");
    assert_eq!(coq_set, isa_set, "Coq and Isabelle IOU axiom sets must agree");
}

// =============================================================================
// Per-foundation IOU axiom arity consistency (PR-1c)
// =============================================================================
//
// PR-1b asserts the axiom *name set* matches across Lean / Coq /
// Isabelle.  PR-1c extends this to *arity* (argument count): for
// each named axiom, all three foundations must declare the same
// number of arguments.  Catches the drift class where a discharge
// removes an argument from one foundation but forgets the others
// — the axiom name still matches but signatures don't.

/// Extract the (name → arity) map from a per-foundation IOU-axiom
/// string constant, given the foundation's argument-separator
/// token (`→` for Lean, `->` for Coq, `\<Rightarrow>` for Isabelle).
///
/// Arity = number of separators in the axiom's signature line.
/// For `A → B → C`: 2 arrows = arity 2 (A and B are args, C is
/// the return type).
fn extract_iou_arities_from_constant(
    constant: &str,
    separator: &str,
) -> std::collections::BTreeMap<String, usize> {
    let mut result = std::collections::BTreeMap::new();
    // Walk line by line — each axiom occupies one line in the
    // emitted constant.
    for line in constant.lines() {
        // Find the first occurrence of `_iou` to extract the name.
        let iou_pos = match line.find("_iou") {
            Some(p) => p,
            None => continue,
        };
        // Walk backwards over identifier chars to find the start.
        let bytes = line.as_bytes();
        let mut start = iou_pos;
        while start > 0 {
            let c = bytes[start - 1];
            if c.is_ascii_alphanumeric() || c == b'_' {
                start -= 1;
            } else {
                break;
            }
        }
        let name_with_iou = &line[start..iou_pos + "_iou".len()];
        if !name_with_iou.starts_with("K_") {
            continue;
        }
        let rule_name = &name_with_iou[..name_with_iou.len() - "_iou".len()];
        // Count the separator occurrences in this line.  For an
        // axiom signature `A → B → C` the arrows separate A from B
        // and B from C; A and B are args, C is the return type, so
        // arity = arrows = 2.
        let separator_count = line.matches(separator).count();
        if separator_count == 0 {
            continue; // not a signature line
        }
        let arity = separator_count;
        result.insert(rule_name.to_string(), arity);
    }
    result
}

#[test]
fn extractor_finds_arities_in_lean_constant() {
    // Sanity: arity extractor returns a map whose size matches the
    // current `iou_axiom_specs()` registry length.  Post-FV-17 the
    // registry is empty (every kernel rule is either Proved or
    // DischargedByFramework with a cited upstream proof) — the
    // extractor produces an empty map and that is the architectural
    // endgame for the IOU-discharge sequence.  The test remains as
    // a regression guard: re-introducing an open IOU adds an
    // entry, and the assertion below catches drift between
    // `iou_axiom_specs()` and the rendered text.
    use crate::soundness::lean::iou_axioms_lean;
    let arities = extract_iou_arities_from_constant(iou_axioms_lean(), "→");
    let expected = crate::soundness::iou_axiom_specs().len();
    assert_eq!(
        arities.len(),
        expected,
        "extractor saw {} axioms; iou_axiom_specs() declares {}",
        arities.len(),
        expected,
    );
}

#[test]
fn lean_coq_arities_agree() {
    // Pin: every IOU axiom has the same arity in Lean and Coq.
    // Drift class: a discharge removed an arg from one foundation
    // but forgot the other.
    use crate::soundness::coq::iou_axioms_coq;
    use crate::soundness::lean::iou_axioms_lean;
    let lean = extract_iou_arities_from_constant(iou_axioms_lean(), "→");
    let coq = extract_iou_arities_from_constant(iou_axioms_coq(), "->");
    assert_eq!(
        lean, coq,
        "Lean and Coq IOU axiom arities must agree per axiom",
    );
}

#[test]
fn coq_isabelle_arities_agree() {
    // Pin: every IOU axiom has the same arity in Coq and
    // Isabelle.  Isabelle uses `\<Rightarrow>` for its arrow
    // separator (HOL function-type constructor).
    use crate::soundness::coq::iou_axioms_coq;
    use crate::soundness::isabelle::iou_axioms_isabelle;
    let coq = extract_iou_arities_from_constant(iou_axioms_coq(), "->");
    let isa = extract_iou_arities_from_constant(iou_axioms_isabelle(), "\\<Rightarrow>");
    assert_eq!(
        coq, isa,
        "Coq and Isabelle IOU axiom arities must agree per axiom",
    );
}

#[test]
fn three_foundations_agree_on_iou_axiom_arities() {
    // Pin: direct three-way arity agreement.  Combines the
    // pairwise pins above into a single canonical assertion that's
    // the natural extension of `three_foundations_agree_on_iou_axiom_set`.
    use crate::soundness::coq::iou_axioms_coq;
    use crate::soundness::isabelle::iou_axioms_isabelle;
    use crate::soundness::lean::iou_axioms_lean;
    let lean = extract_iou_arities_from_constant(iou_axioms_lean(), "→");
    let coq = extract_iou_arities_from_constant(iou_axioms_coq(), "->");
    let isa = extract_iou_arities_from_constant(iou_axioms_isabelle(), "\\<Rightarrow>");
    assert_eq!(lean, coq);
    assert_eq!(coq, isa);
    // Sanity: the maps' size matches the registry exactly.  Post-FV-17
    // this is zero by design (every kernel rule is structurally Proved
    // or DischargedByFramework).
    let expected = crate::soundness::iou_axiom_specs().len();
    assert_eq!(
        lean.len(),
        expected,
        "Lean arity map size {} must match `iou_axiom_specs()` length {}",
        lean.len(),
        expected,
    );
}

// =============================================================================
// Source-of-truth IOU axiom arity (PR-1d)
// =============================================================================
//
// PR-1c asserts the three foundations agree on arity, but doesn't
// anchor on a canonical specification — drift could happen
// across all three coherently.  PR-1d adds the canonical anchor:
// `iou_axiom_specs()` returns `Vec<IouAxiomSpec { name, arity }>`,
// and pin tests assert each foundation's parsed arity matches the
// spec.

#[test]
fn iou_axiom_specs_has_one_entry_per_iou_rule_name() {
    // Pin: the spec list and the rule-names list have the same
    // length (one spec per rule).  Catches asymmetric edits between
    // the two source-of-truth surfaces (e.g. adding a rule name
    // without a corresponding spec arity).
    use crate::soundness::{iou_axiom_rule_names, iou_axiom_specs};
    let specs = iou_axiom_specs();
    let names = iou_axiom_rule_names();
    assert_eq!(specs.len(), names.len());
    // Order pin: derived rule_names() preserves spec order.
    let spec_names: Vec<&str> = specs.iter().map(|s| s.rule_name).collect();
    assert_eq!(spec_names, names);
}

#[test]
fn iou_axiom_specs_arities_are_positive() {
    // Pin: every IOU axiom has at least 1 arrow (Ctx → Prop is
    // the minimum shape).  Arity 0 would mean a Prop literal,
    // which doesn't fit the IOU axiom template.
    use crate::soundness::iou_axiom_specs;
    for spec in iou_axiom_specs() {
        assert!(
            spec.arity >= 1,
            "rule {} has arity {} — must be ≥ 1",
            spec.rule_name,
            spec.arity,
        );
    }
}

#[test]
fn lean_constant_arities_match_source_of_truth() {
    // Pin: every IOU axiom in iou_axioms_lean() has the arity
    // declared by `iou_axiom_specs()`.  Drift here means the Lean
    // signature was edited (added/removed an arg) but the spec
    // wasn't updated to match — or vice versa.
    use crate::soundness::iou_axiom_specs;
    use crate::soundness::lean::iou_axioms_lean;
    let parsed = extract_iou_arities_from_constant(iou_axioms_lean(), "→");
    for spec in iou_axiom_specs() {
        let actual = parsed.get(spec.rule_name).copied().unwrap_or(0);
        assert_eq!(
            actual, spec.arity,
            "Lean axiom {}_iou has parsed arity {} but spec declares {}",
            spec.rule_name, actual, spec.arity,
        );
    }
}

#[test]
fn coq_constant_arities_match_source_of_truth() {
    // Pin: every IOU axiom in iou_axioms_coq() matches `iou_axiom_specs()`.
    use crate::soundness::coq::iou_axioms_coq;
    use crate::soundness::iou_axiom_specs;
    let parsed = extract_iou_arities_from_constant(iou_axioms_coq(), "->");
    for spec in iou_axiom_specs() {
        let actual = parsed.get(spec.rule_name).copied().unwrap_or(0);
        assert_eq!(
            actual, spec.arity,
            "Coq axiom {}_iou has parsed arity {} but spec declares {}",
            spec.rule_name, actual, spec.arity,
        );
    }
}

#[test]
fn isabelle_constant_arities_match_source_of_truth() {
    // Pin: every IOU axiom in iou_axioms_isabelle() matches `iou_axiom_specs()`.
    use crate::soundness::iou_axiom_specs;
    use crate::soundness::isabelle::iou_axioms_isabelle;
    let parsed = extract_iou_arities_from_constant(iou_axioms_isabelle(), "\\<Rightarrow>");
    for spec in iou_axiom_specs() {
        let actual = parsed.get(spec.rule_name).copied().unwrap_or(0);
        assert_eq!(
            actual, spec.arity,
            "Isabelle axiom {}_iou has parsed arity {} but spec declares {}",
            spec.rule_name, actual, spec.arity,
        );
    }
}

// =============================================================================
// Verum-side theorems.vr ↔ Rust mod.rs status parity (PR-1e)
// =============================================================================
//
// The .vr corpus at `core/verify/kernel_soundness/theorems.vr`
// carries `lemma_status(KernelRule.K<Name>) => LemmaStatus.<X>`
// per-rule entries.  This was synced manually in PR-5e + PR-5g +
// PR-5h but had no automatic check — drift surface uncovered.
// PR-1e closes the loop: parse the .vr file, assert per-rule
// status agreement with `canonical_rules()`.

/// Status keyword (`Proved` / `Admitted` / `DischargedByFramework`).
/// Stored as a stable string for lightweight comparison.
type VrStatusKind = &'static str;

/// Parse `KernelRule.K<Name> => LemmaStatus.<Status>` lines from a
/// .vr file.  Returns a map from the .vr-format rule name (no
/// underscores, e.g. `KQuotElim`) to the status keyword.  Robust
/// to surrounding whitespace and the optional `{ reason: ... }` /
/// `{ lemma_path: ..., framework: ..., citation: ... }` body
/// after the status keyword.
fn parse_vr_lemma_status(
    text: &str,
) -> std::collections::BTreeMap<String, VrStatusKind> {
    let mut result: std::collections::BTreeMap<String, VrStatusKind> =
        std::collections::BTreeMap::new();
    for line in text.lines() {
        // Strip leading whitespace.
        let trimmed = line.trim_start();
        // Match `KernelRule.K<Name> => LemmaStatus.<Status>`.
        let prefix = "KernelRule.K";
        if !trimmed.starts_with(prefix) {
            continue;
        }
        let rest = &trimmed[prefix.len()..];
        // Walk identifier chars (alphanumeric / `_`) — stop at the
        // first non-ident char.  This collects the `<Name>` part.
        let name_end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        if name.is_empty() {
            continue;
        }
        let after_name = &rest[name_end..].trim_start();
        // Expect `=>`.
        if !after_name.starts_with("=>") {
            continue;
        }
        let after_arrow = after_name[2..].trim_start();
        // Expect `LemmaStatus.<Status>`.
        let status_prefix = "LemmaStatus.";
        if !after_arrow.starts_with(status_prefix) {
            continue;
        }
        let status_rest = &after_arrow[status_prefix.len()..];
        // Walk identifier chars to extract status name.
        let status_end = status_rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(status_rest.len());
        let status: VrStatusKind = match &status_rest[..status_end] {
            "Proved" => "Proved",
            "Admitted" => "Admitted",
            "DischargedByFramework" => "DischargedByFramework",
            other => panic!(
                "unexpected LemmaStatus variant '{}' in .vr file",
                other
            ),
        };
        // Re-prepend `K` (the prefix we stripped earlier).
        let full_name = format!("K{}", name);
        result.insert(full_name, status);
    }
    result
}

/// Convert a Rust rule name (snake-cased like `K_Quot_Elim`) to
/// the .vr camelCase form (`KQuotElim`).  Rule: drop all
/// underscores.  Verified by inspection against the 38 entries in
/// `canonical_rules()`.
fn rust_rule_name_to_vr(rust: &str) -> String {
    rust.replace('_', "")
}

#[test]
fn rust_to_vr_name_conversion_round_trip() {
    // Pin: the conversion is correct on every canonical rule
    // name.  Catches a future rule name with non-underscore
    // word-break punctuation that breaks the simple replace.
    let exporter = SoundnessExporter::new();
    for rule in exporter.rules() {
        let vr = rust_rule_name_to_vr(&rule.rule_name);
        // Sanity: the .vr name starts with K and is alphanumeric.
        assert!(
            vr.starts_with('K') && vr.chars().all(|c| c.is_ascii_alphanumeric()),
            "{} → {} fails the .vr-name-shape pin",
            rule.rule_name,
            vr,
        );
    }
}

#[test]
fn vr_corpus_has_one_entry_per_kernel_rule() {
    // Pin: theorems.vr declares exactly 38 lemma_status entries
    // — one per kernel rule.
    let vr_text = include_str!(
        "../../../../core/verify/kernel_soundness/theorems.vr"
    );
    let parsed = parse_vr_lemma_status(vr_text);
    assert_eq!(
        parsed.len(),
        EXPECTED_KERNEL_RULE_COUNT,
        "theorems.vr should have exactly {} lemma_status entries; got {}",
        EXPECTED_KERNEL_RULE_COUNT,
        parsed.len(),
    );
}

#[test]
fn vr_corpus_status_matches_rust_mod_rs() {
    // Pin: per-rule LemmaStatus parity between Rust mod.rs and
    // .vr theorems.vr.  Catches manual sync omissions before
    // they accumulate.
    let vr_text = include_str!(
        "../../../../core/verify/kernel_soundness/theorems.vr"
    );
    let vr_status_map = parse_vr_lemma_status(vr_text);

    let exporter = SoundnessExporter::new();
    let mut errors: Vec<String> = Vec::new();
    for rule in exporter.rules() {
        let vr_name = rust_rule_name_to_vr(&rule.rule_name);
        let vr_status = match vr_status_map.get(&vr_name) {
            Some(s) => *s,
            None => {
                errors.push(format!(
                    "rule {} (vr: {}) has no entry in theorems.vr",
                    rule.rule_name, vr_name,
                ));
                continue;
            }
        };
        let rust_status: VrStatusKind = match rule.status {
            LemmaStatus::Proved { .. } => "Proved",
            LemmaStatus::Admitted { .. } => "Admitted",
            LemmaStatus::DischargedByFramework { .. } => "DischargedByFramework",
        };
        if rust_status != vr_status {
            errors.push(format!(
                "rule {} drift: mod.rs={}, theorems.vr={}",
                rule.rule_name, rust_status, vr_status,
            ));
        }
    }
    assert!(
        errors.is_empty(),
        "Verum-side theorems.vr drift from Rust mod.rs:\n{}",
        errors.join("\n"),
    );
}

// =============================================================================
// Citation-parity for DischargedByFramework rules
// =============================================================================
//
// `vr_corpus_status_matches_rust_mod_rs` checks the status keyword
// only.  Extend coverage to the citation triple
// `(lemma_path, framework, citation)` for `DischargedByFramework`
// rules — the framework attribution can drift independently of the
// status keyword.

/// Normalize a string by collapsing all whitespace runs to a single
/// space and trimming.  Used to compare strings that may have
/// different line-continuation formatting between Rust source and
/// .vr source.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parsed citation triple for a DischargedByFramework rule.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VrCitation {
    lemma_path: String,
    framework: String,
    citation: String,
}

/// Extract `Text.from("…")` body — handles both single-line and
/// multi-line forms.  Returns the literal string content (stripped
/// of surrounding quotes and `\`-newline-whitespace continuations).
fn extract_text_from_arg(text: &str, key: &str) -> Option<String> {
    // Find the key followed by `: Text.from(`.
    let needle = format!("{}: Text.from(", key);
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    // Find the opening `"`.
    let q1 = rest.find('"')? + 1;
    let after_q1 = &rest[q1..];
    // Scan forward for the closing `"` that isn't escaped.  The
    // .vr source uses `\<newline><whitespace>` continuations
    // *inside* the string (Verum's line-continuation in strings),
    // and double-quotes inside the body would be escaped — but
    // none of the actual citation strings contain `\"`.  So a
    // simple scan for the next `"` works.
    let mut depth = 0;
    let bytes = after_q1.as_bytes();
    while depth < bytes.len() {
        if bytes[depth] == b'"' {
            // Found closing quote.  The body is bytes[..depth].
            let body = &after_q1[..depth];
            // Strip `\<newline><ws>` continuations: replace each
            // such sequence with a single space.
            let mut cleaned = String::with_capacity(body.len());
            let mut chars = body.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\\' && chars.peek() == Some(&'\n') {
                    chars.next(); // consume the \n
                    // Skip leading whitespace on the next line.
                    while let Some(&c2) = chars.peek() {
                        if c2 == ' ' || c2 == '\t' {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    cleaned.push(' '); // continuation = one space
                } else {
                    cleaned.push(c);
                }
            }
            return Some(cleaned);
        }
        if bytes[depth] == b'\\' && depth + 1 < bytes.len() {
            depth += 2;
            continue;
        }
        depth += 1;
    }
    None
}

/// Parse `KernelRule.K<Name> => LemmaStatus.DischargedByFramework
/// { lemma_path: …, framework: …, citation: …, }` blocks from the
/// .vr file.  Returns a map from .vr-format rule name to parsed
/// citation triple.
fn parse_vr_discharged_citations(
    text: &str,
) -> std::collections::BTreeMap<String, VrCitation> {
    let mut result = std::collections::BTreeMap::new();
    // Find each `KernelRule.K<Name> => LemmaStatus.DischargedByFramework {`
    // header, then the matching closing `},`.
    let header_pattern = "=> LemmaStatus.DischargedByFramework {";
    let mut search_start = 0;
    while let Some(header_pos) = text[search_start..].find(header_pattern) {
        let abs_header = search_start + header_pos;
        // Walk back to find the rule name `KernelRule.K<Name>`.
        // The pattern: `KernelRule.K<Name> ` precedes `=>`.
        let prefix = &text[..abs_header];
        let kernel_rule_marker = "KernelRule.K";
        let krm_pos = match prefix.rfind(kernel_rule_marker) {
            Some(p) => p,
            None => {
                search_start = abs_header + header_pattern.len();
                continue;
            }
        };
        let after_krm = &text[krm_pos + kernel_rule_marker.len()..abs_header];
        // Walk forward over identifier chars to extract the name.
        let name_end = after_krm
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(after_krm.len());
        let name = &after_krm[..name_end];
        if name.is_empty() {
            search_start = abs_header + header_pattern.len();
            continue;
        }
        let vr_name = format!("K{}", name);

        // Find the matching closing brace.  The body is between
        // the `{` after the header and the next balanced `},` or
        // `}\n`.  Use brace counting.
        let body_start = abs_header + header_pattern.len();
        let body_bytes = text.as_bytes();
        let mut brace_depth = 1;
        let mut body_end = body_start;
        while body_end < body_bytes.len() && brace_depth > 0 {
            match body_bytes[body_end] {
                b'{' => brace_depth += 1,
                b'}' => brace_depth -= 1,
                _ => {}
            }
            body_end += 1;
        }
        if brace_depth != 0 {
            break; // malformed
        }
        let body = &text[body_start..body_end - 1]; // exclude closing `}`

        if let (Some(lp), Some(fw), Some(c)) = (
            extract_text_from_arg(body, "lemma_path"),
            extract_text_from_arg(body, "framework"),
            extract_text_from_arg(body, "citation"),
        ) {
            result.insert(
                vr_name,
                VrCitation {
                    lemma_path: lp,
                    framework: fw,
                    citation: c,
                },
            );
        }
        search_start = body_end;
    }
    result
}

#[test]
fn extract_text_from_arg_handles_single_line() {
    let body = r#"
        lemma_path: Text.from("core.verify.kernel_v0.lemmas.subst.subst_preserves_typing"),
        framework: Text.from("mathlib4"),
        citation: Text.from("Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing"),
    "#;
    assert_eq!(
        extract_text_from_arg(body, "framework"),
        Some("mathlib4".to_string()),
    );
    assert_eq!(
        extract_text_from_arg(body, "lemma_path"),
        Some("core.verify.kernel_v0.lemmas.subst.subst_preserves_typing".to_string()),
    );
}

#[test]
fn vr_corpus_has_nine_discharged_by_framework_entries() {
    // Pin: theorems.vr declares exactly 9 DischargedByFramework
    // entries — matches the count of such rules in
    // canonical_rules() after FV-17 added K_Eps_Mu and K_Round_Trip
    // to the bucket: K_Pi_Form / K_Lam_Intro / K_App_Elim /
    // K_Sigma_Form / K_Pair_Intro / K_Fst_Elim / K_Snd_Elim /
    // K_Eps_Mu / K_Round_Trip.
    let vr_text = include_str!(
        "../../../../core/verify/kernel_soundness/theorems.vr"
    );
    let citations = parse_vr_discharged_citations(vr_text);
    assert_eq!(
        citations.len(),
        9,
        "expected 9 DischargedByFramework entries in theorems.vr; got {}",
        citations.len(),
    );
}

// =============================================================================
// Per-position arg-type agreement across foundations
// =============================================================================
//
// Existing arity drift guard verifies all three foundations agree
// on the number of arguments per axiom, but doesn't catch the
// drift class where one foundation has the args in a different
// position than another (e.g. K_Refine_Intro_iou: same arity 5
// but [CoreTerm, CoreTerm, String, CoreTerm] in Lean vs
// [CoreTerm, String, CoreTerm, CoreTerm] in Coq).  This drift
// would silently change which positional argument the IOU
// witnesses; soundness lemmas using positional pattern matching
// would type-check vacuously.

/// Extract the type-name sequence per axiom from a foundation's
/// IOU-axiom string constant.  Returns map from rule_name to a
/// `Vec<String>` of type tokens (excluding `Ctx` and the return
/// type), normalized to lowercase for cross-foundation comparison
/// (Lean uses `CoreTerm`/`String`/`Nat`, Coq uses
/// `CoreTerm`/`string`/`nat`, Isabelle uses `CoreTerm`/`string`/
/// `nat`).
fn extract_iou_arg_types_from_constant(
    constant: &str,
    separator: &str,
) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut result = std::collections::BTreeMap::new();
    for line in constant.lines() {
        let iou_pos = match line.find("_iou") {
            Some(p) => p,
            None => continue,
        };
        let bytes = line.as_bytes();
        let mut start = iou_pos;
        while start > 0 {
            let c = bytes[start - 1];
            if c.is_ascii_alphanumeric() || c == b'_' {
                start -= 1;
            } else {
                break;
            }
        }
        let name_with_iou = &line[start..iou_pos + "_iou".len()];
        if !name_with_iou.starts_with("K_") {
            continue;
        }
        let rule_name = &name_with_iou[..name_with_iou.len() - "_iou".len()];
        // Find the part after `:` (Lean/Coq) or `::` (Isabelle).
        // For Isabelle, the type expression is enclosed in `"..."`;
        // strip the quotes.  For Lean/Coq, the expression continues
        // to end of line.
        let after_colon_idx = match line.find("::") {
            Some(p) => p + 2,
            None => match line.find(": ") {
                Some(p) => p + 2,
                None => continue,
            },
        };
        let mut sig = line[after_colon_idx..].trim().to_string();
        // Strip surrounding quotes (Isabelle).
        if sig.starts_with('"') {
            if let Some(end) = sig[1..].find('"') {
                sig = sig[1..=end].to_string();
            }
        }
        // Split by the separator.  Trim each piece.  Skip empty.
        let pieces: Vec<&str> = sig.split(separator).collect();
        if pieces.len() < 2 {
            continue; // not a real signature
        }
        // First arg is `Ctx`, last is `Prop` / `bool` — drop both.
        // Middle pieces are the rule-specific arg types.
        let arg_types: Vec<String> = pieces[1..pieces.len() - 1]
            .iter()
            .map(|s| s.trim().trim_end_matches(',').to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        result.insert(rule_name.to_string(), arg_types);
    }
    result
}

#[test]
fn extractor_finds_arg_types_in_lean() {
    // Post-FV-17 the IOU registry is empty — the extractor produces
    // an empty map.  Regression guard: re-introducing an open IOU
    // populates the map, and the size assertion below catches drift
    // between `iou_axiom_specs()` and the rendered text.
    use crate::soundness::lean::iou_axioms_lean;
    let arg_types = extract_iou_arg_types_from_constant(iou_axioms_lean(), "→");
    let expected = crate::soundness::iou_axiom_specs().len();
    assert_eq!(
        arg_types.len(),
        expected,
        "extractor saw {} axioms; iou_axiom_specs() declares {}",
        arg_types.len(),
        expected,
    );
}

#[test]
fn lean_coq_arg_types_agree_per_position() {
    use crate::soundness::coq::iou_axioms_coq;
    use crate::soundness::lean::iou_axioms_lean;
    let lean = extract_iou_arg_types_from_constant(iou_axioms_lean(), "→");
    let coq = extract_iou_arg_types_from_constant(iou_axioms_coq(), "->");
    assert_eq!(
        lean, coq,
        "Lean and Coq arg-type sequences must match per position",
    );
}

#[test]
fn coq_isabelle_arg_types_agree_per_position() {
    use crate::soundness::coq::iou_axioms_coq;
    use crate::soundness::isabelle::iou_axioms_isabelle;
    let coq = extract_iou_arg_types_from_constant(iou_axioms_coq(), "->");
    let isa = extract_iou_arg_types_from_constant(iou_axioms_isabelle(), "\\<Rightarrow>");
    assert_eq!(
        coq, isa,
        "Coq and Isabelle arg-type sequences must match per position",
    );
}

#[test]
fn three_foundations_agree_on_iou_axiom_arg_types() {
    // Pin: direct three-way arg-type-sequence agreement.  This
    // catches the drift class where one foundation permutes args
    // (same arity, different positional order) — invisible to
    // arity-only checks.
    use crate::soundness::coq::iou_axioms_coq;
    use crate::soundness::isabelle::iou_axioms_isabelle;
    use crate::soundness::lean::iou_axioms_lean;
    let lean = extract_iou_arg_types_from_constant(iou_axioms_lean(), "→");
    let coq = extract_iou_arg_types_from_constant(iou_axioms_coq(), "->");
    let isa = extract_iou_arg_types_from_constant(iou_axioms_isabelle(), "\\<Rightarrow>");
    assert_eq!(lean, coq);
    assert_eq!(coq, isa);
    // Sanity: every spec'd rule has a non-empty arg-type sequence.
    use crate::soundness::iou_axiom_specs;
    for spec in iou_axiom_specs() {
        let types = lean.get(spec.rule_name);
        assert!(
            types.is_some(),
            "rule {} has no arg-type sequence parsed from Lean",
            spec.rule_name,
        );
        // The parsed length should equal the spec's arity minus 1
        // (we drop Ctx but keep the other args).
        assert_eq!(
            types.unwrap().len(),
            spec.arity - 1,
            "rule {} arg-type sequence length {} ≠ spec arity {} - 1",
            spec.rule_name,
            types.unwrap().len(),
            spec.arity,
        );
    }
}

#[test]
fn vr_corpus_citation_triples_match_rust_mod_rs() {
    // Pin: per-rule citation triple parity between mod.rs and
    // theorems.vr.  Catches drift in the (lemma_path, framework,
    // citation) attribution that the status-keyword check missed.
    let vr_text = include_str!(
        "../../../../core/verify/kernel_soundness/theorems.vr"
    );
    let vr_citations = parse_vr_discharged_citations(vr_text);

    let exporter = SoundnessExporter::new();
    let mut errors: Vec<String> = Vec::new();
    for rule in exporter.rules() {
        let (rust_lp, rust_fw, rust_cit) = match &rule.status {
            LemmaStatus::DischargedByFramework {
                lemma_path,
                framework,
                citation,
            } => (lemma_path, framework, citation),
            _ => continue, // not DischargedByFramework — skip
        };
        let vr_name = rust_rule_name_to_vr(&rule.rule_name);
        let vr_cite = match vr_citations.get(&vr_name) {
            Some(c) => c,
            None => {
                errors.push(format!(
                    "rule {} (vr: {}) is DischargedByFramework in mod.rs but \
                     has no DischargedByFramework entry in theorems.vr",
                    rule.rule_name, vr_name,
                ));
                continue;
            }
        };
        if normalize_ws(rust_lp) != normalize_ws(&vr_cite.lemma_path) {
            errors.push(format!(
                "rule {} lemma_path drift:\n  mod.rs: {}\n  vr:     {}",
                rule.rule_name,
                normalize_ws(rust_lp),
                normalize_ws(&vr_cite.lemma_path),
            ));
        }
        if normalize_ws(rust_fw) != normalize_ws(&vr_cite.framework) {
            errors.push(format!(
                "rule {} framework drift: mod.rs={}, vr={}",
                rule.rule_name,
                normalize_ws(rust_fw),
                normalize_ws(&vr_cite.framework),
            ));
        }
        if normalize_ws(rust_cit) != normalize_ws(&vr_cite.citation) {
            errors.push(format!(
                "rule {} citation drift:\n  mod.rs: {}\n  vr:     {}",
                rule.rule_name,
                normalize_ws(rust_cit),
                normalize_ws(&vr_cite.citation),
            ));
        }
    }
    assert!(
        errors.is_empty(),
        "Verum-side theorems.vr citation drift from Rust mod.rs:\n{}",
        errors.join("\n"),
    );
}
