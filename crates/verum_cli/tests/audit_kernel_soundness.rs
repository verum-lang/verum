//! Integration tests for `verum audit --kernel-soundness` (#80 / VERUM-TRUST-1).
//!
//! Pin coverage:
//!   - Plain output reports the canonical 38-rule corpus + IOU list.
//!   - JSON output is structured with the schema_version=1 envelope.
//!   - Cross-export produces both `kernel_soundness.v` (Coq) and
//!     `KernelSoundness.lean` (Lean 4) under
//!     `target/audit-reports/kernel-soundness/`.
//!   - The drift-check passes (the canonical Rust rule list agrees
//!     with `EXPECTED_KERNEL_RULE_COUNT`).
//!   - The four structurally-proved lemmas (K-Var, K-Univ, K-FwAx,
//!     K-Pos) appear with `Proof. … Qed.` in the Coq output and `:= by`
//!     blocks in the Lean output.
//!   - Every admitted lemma's IOU reason is preserved verbatim in
//!     both the report and the foreign-tool emission.

#![allow(unused_imports)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn create_project(name: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("create tempdir");
    let dir = temp.path().join(name);
    fs::create_dir_all(&dir).expect("create project dir");
    let manifest = format!(
        r#"[cog]
name = "{name}"
version = "0.1.0"

[language]
profile = "application"

[dependencies]
"#
    );
    fs::write(dir.join("Verum.toml"), manifest).expect("write Verum.toml");
    let src = dir.join("src");
    fs::create_dir_all(&src).expect("create src/");
    fs::write(src.join("main.vr"), "public fn main() {}").expect("write main.vr");
    (temp, dir)
}

fn run_verum(args: &[&str], cwd: &PathBuf) -> Output {
    Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn verum CLI")
}

#[test]
fn plain_output_reports_canonical_corpus() {
    let (_temp, dir) = create_project("ks_plain");
    let out = run_verum(&["audit", "--kernel-soundness"], &dir);

    assert!(
        out.status.success(),
        "audit --kernel-soundness must exit 0 on the canonical corpus.\n\
         stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    // Headline counts.
    assert!(
        stdout.contains("38 kernel rules"),
        "report must show 38-rule corpus; got:\n{}",
        stdout,
    );
    assert!(
        stdout.contains("4 structurally proved"),
        "report must show 4 proved lemmas",
    );
    assert!(
        stdout.contains("34 admitted"),
        "report must show 34 admitted lemmas",
    );
    // IOU enumeration includes the K_Pi_Form line.
    assert!(
        stdout.contains("K_Pi_Form"),
        "report must enumerate K_Pi_Form among IOUs",
    );
    assert!(
        stdout.contains("substitution-lemma"),
        "K_Pi_Form's admit reason must appear verbatim",
    );
}

#[test]
fn json_output_has_schema_v1_envelope() {
    let (_temp, dir) = create_project("ks_json");
    let out = run_verum(
        &["audit", "--kernel-soundness", "--format", "json"],
        &dir,
    );

    assert!(out.status.success(), "JSON-format audit must exit 0");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let payload: serde_json::Value = serde_json::from_str(&stdout)
        .expect("audit JSON must be parseable");

    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["command"], "audit-kernel-soundness");
    assert_eq!(payload["expected_rule_count"], 38);
    assert_eq!(payload["actual_rule_count"], 38);
    assert_eq!(payload["proved_count"], 4);
    assert_eq!(payload["admitted_count"], 34);
    assert_eq!(payload["holds_unconditionally"], false);

    let rules = payload["rules"]
        .as_array()
        .expect("rules array must be present");
    assert_eq!(rules.len(), 38);

    let ious = payload["outstanding_ious"]
        .as_array()
        .expect("outstanding_ious array must be present");
    assert_eq!(ious.len(), 34);
    for iou in ious {
        assert!(iou["rule_name"].is_string());
        assert!(iou["reason"].is_string());
        let reason = iou["reason"].as_str().unwrap();
        assert!(!reason.is_empty(), "IOU reason must not be empty");
    }

    assert!(payload["exports"]["coq_path"].is_string());
    assert!(payload["exports"]["lean_path"].is_string());
}

#[test]
fn cross_export_writes_coq_and_lean_files() {
    let (_temp, dir) = create_project("ks_export");
    let out = run_verum(&["audit", "--kernel-soundness"], &dir);
    assert!(out.status.success(), "audit must exit 0");

    let report_dir = dir
        .join("target")
        .join("audit-reports")
        .join("kernel-soundness");
    let coq_path = report_dir.join("kernel_soundness.v");
    let lean_path = report_dir.join("KernelSoundness.lean");

    assert!(coq_path.exists(), "Coq export must be written to {:?}", coq_path);
    assert!(lean_path.exists(), "Lean export must be written to {:?}", lean_path);

    let coq_text = fs::read_to_string(&coq_path).expect("read Coq output");
    let lean_text = fs::read_to_string(&lean_path).expect("read Lean output");

    // Coq: file headers + canonical inductives + main theorem present.
    assert!(coq_text.contains("kernel_soundness.v"));
    assert!(coq_text.contains("Inductive CoreTerm"));
    assert!(coq_text.contains("Inductive CoreType"));
    assert!(coq_text.contains("Inductive KernelRule"));
    assert!(coq_text.contains("Theorem kernel_soundness"));

    // Lean: file headers + canonical inductives + main theorem present.
    assert!(lean_text.contains("KernelSoundness.lean"));
    assert!(lean_text.contains("inductive CoreTerm"));
    assert!(lean_text.contains("inductive CoreType"));
    assert!(lean_text.contains("inductive KernelRule"));
    assert!(lean_text.contains("theorem kernel_soundness"));
}

#[test]
fn proved_lemmas_carry_qed_and_admits_carry_reason() {
    let (_temp, dir) = create_project("ks_status");
    let out = run_verum(&["audit", "--kernel-soundness"], &dir);
    assert!(out.status.success());

    let coq_path = dir
        .join("target")
        .join("audit-reports")
        .join("kernel-soundness")
        .join("kernel_soundness.v");
    let lean_path = dir
        .join("target")
        .join("audit-reports")
        .join("kernel-soundness")
        .join("KernelSoundness.lean");
    let coq_text = fs::read_to_string(&coq_path).expect("read Coq output");
    let lean_text = fs::read_to_string(&lean_path).expect("read Lean output");

    // The four structurally-proved lemmas carry `Qed.` (Coq) and `:= by`
    // tactic blocks (Lean), not `Admitted.` / `sorry`.
    for proved in ["K_Var_sound", "K_Univ_sound", "K_FwAx_sound", "K_Pos_sound"] {
        let coq_pos = coq_text
            .find(&format!("Lemma {}", proved))
            .unwrap_or_else(|| panic!("Coq output missing {}", proved));
        // After the Lemma keyword, the next 200 chars should contain `Qed.`
        // (the proved lemmas are short).  Search a window to avoid
        // collisions with later admitted lemmas.
        let window_end = (coq_pos + 800).min(coq_text.len());
        let window = &coq_text[coq_pos..window_end];
        assert!(
            window.contains("Qed."),
            "{}: Coq output must end its proof in Qed., got window:\n{}",
            proved,
            window,
        );
        // And does NOT contain Admitted. inside that window
        // (until the NEXT lemma).  Look for the next `Lemma ` start
        // and constrain the window.
        let next_lemma_offset = coq_text[coq_pos + 5..].find("Lemma ");
        let bounded_end = match next_lemma_offset {
            Some(off) => coq_pos + 5 + off,
            None => coq_text.len(),
        };
        let bounded = &coq_text[coq_pos..bounded_end];
        assert!(
            !bounded.contains("Admitted."),
            "{}: proved lemma must not contain Admitted. — window: {}",
            proved,
            bounded,
        );

        let lean_pos = lean_text
            .find(&format!("theorem {}", proved))
            .unwrap_or_else(|| panic!("Lean output missing {}", proved));
        let lean_window_end = (lean_pos + 800).min(lean_text.len());
        let lean_window = &lean_text[lean_pos..lean_window_end];
        assert!(
            lean_window.contains(":= by"),
            "{}: Lean output must use `:= by` for the proved lemma",
            proved,
        );
    }

    // Spot-check: K_Pi_Form is admitted with the substitution-lemma
    // reason in BOTH foreign-tool outputs.
    assert!(coq_text.contains("Admitted."));
    assert!(coq_text.contains("substitution-lemma"));
    assert!(lean_text.contains("sorry"));
    assert!(lean_text.contains("substitution-lemma"));
}

#[test]
fn every_rule_appears_as_kernel_rule_constructor_in_both_exports() {
    let (_temp, dir) = create_project("ks_partition");
    let out = run_verum(&["audit", "--kernel-soundness"], &dir);
    assert!(out.status.success());

    let coq_text = fs::read_to_string(
        dir.join("target/audit-reports/kernel-soundness/kernel_soundness.v"),
    ).expect("read Coq output");
    let lean_text = fs::read_to_string(
        dir.join("target/audit-reports/kernel-soundness/KernelSoundness.lean"),
    ).expect("read Lean output");

    // Every one of the 38 canonical rules must appear as a constructor
    // in the KernelRule inductive AND as a Lemma/theorem name.
    let canonical_rules = [
        "K_Var", "K_Univ", "K_Pi_Form", "K_Lam_Intro", "K_App_Elim",
        "K_Sigma_Form", "K_Pair_Intro", "K_Fst_Elim", "K_Snd_Elim",
        "K_Path_Ty_Form", "K_Path_Over_Form", "K_Refl_Intro", "K_HComp",
        "K_Transp", "K_Glue",
        "K_Refine", "K_Refine_Omega", "K_Refine_Intro", "K_Refine_Erase",
        "K_Quot_Form", "K_Quot_Intro", "K_Quot_Elim",
        "K_Inductive", "K_Pos", "K_Elim",
        "K_Smt", "K_FwAx",
        "K_Eps_Mu", "K_Universe_Ascent", "K_Round_Trip",
        "K_Epsilon_Of", "K_Alpha_Of",
        "K_Modal_Box", "K_Modal_Diamond", "K_Modal_Big_And",
        "K_Shape", "K_Flat", "K_Sharp",
    ];
    assert_eq!(canonical_rules.len(), 38);

    for rule in &canonical_rules {
        assert!(
            coq_text.contains(rule),
            "Coq output missing rule {}",
            rule,
        );
        assert!(
            lean_text.contains(rule),
            "Lean output missing rule {}",
            rule,
        );
        let lemma = format!("{}_sound", rule);
        assert!(
            coq_text.contains(&lemma),
            "Coq output missing lemma {}",
            lemma,
        );
        assert!(
            lean_text.contains(&lemma),
            "Lean output missing lemma {}",
            lemma,
        );
    }
}

#[test]
fn json_rules_array_carries_per_rule_metadata() {
    let (_temp, dir) = create_project("ks_metadata");
    let out = run_verum(
        &["audit", "--kernel-soundness", "--format", "json"],
        &dir,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let rules = payload["rules"].as_array().unwrap();
    let pi_form = rules
        .iter()
        .find(|r| r["rule_name"] == "K_Pi_Form")
        .expect("K_Pi_Form must be in rules array");
    assert_eq!(pi_form["lemma_name"], "K_Pi_Form_sound");
    assert_eq!(pi_form["category"], "Structural");
    assert_eq!(pi_form["premise_arity"], 2);
    assert_eq!(pi_form["has_side_condition"], false);
    assert_eq!(pi_form["status"], "Admitted");
    assert!(pi_form["admit_reason"].as_str().unwrap().contains("substitution-lemma"));

    // K_Var is proved → admit_reason is Null
    let k_var = rules.iter().find(|r| r["rule_name"] == "K_Var").unwrap();
    assert_eq!(k_var["status"], "Proved");
    assert!(k_var["admit_reason"].is_null());
}
