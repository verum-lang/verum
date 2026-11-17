//! Packaged audit reports distinguish schema proofs from checker correspondence.
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;
use verum_kernel::soundness::{EXPECTED_KERNEL_RULE_COUNT, SoundnessExporter};

fn project() -> TempDir {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("Verum.toml"), "[cog]\nname = \"schema-audit\"\nversion = \"0.1.0\"\n[language]\nprofile = \"application\"\n[dependencies]\n").unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.vr"), "public fn main() {}").unwrap();
    temp
}
fn audit(root: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .arg("audit")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn plain_report_exposes_unsupported_rules_and_scope() {
    let root = project();
    let text = audit(root.path(), &["--kernel-soundness"]);
    assert!(text.contains(&format!("{} kernel rules", EXPECTED_KERNEL_RULE_COUNT)));
    assert!(text.contains("Unavailable K_Smt"));
    assert!(text.contains("abstract admission schemas"));
    assert!(text.contains("Rust checker correspondence is not established"));
    assert!(text.contains("External proof tools were not run"));
    assert!(!text.contains("UNCONDITIONALLY"));
}
#[test]
fn json_report_distinguishes_schemas_from_implementation_correspondence() {
    let root = project();
    let report: serde_json::Value = serde_json::from_str(&audit(
        root.path(),
        &["--kernel-soundness", "--format", "json"],
    ))
    .unwrap();
    let exporter = SoundnessExporter::new();
    assert_eq!(report["schema_version"], 2);
    assert_eq!(report["actual_rule_count"], EXPECTED_KERNEL_RULE_COUNT);
    assert_eq!(report["expected_rule_count"], EXPECTED_KERNEL_RULE_COUNT);
    assert_eq!(report["proved_count"], exporter.proved_count());
    assert_eq!(report["admitted_count"], exporter.admitted_count());
    assert_eq!(report["unsupported_count"], exporter.unsupported_count());
    assert!(exporter.unsupported_count() > 0);
    assert_eq!(report["scope"], "abstract_admission_schemas");
    assert_eq!(report["implementation_correspondence"], "not_established");
    assert_eq!(report["external_replay"], "not_run");
    assert!(report.get("holds_unconditionally").is_none());
    assert_eq!(
        report["backend_models"]["agda"]["typing_schema_derivations"],
        0
    );
    assert_eq!(
        report["backend_models"]["isabelle"]["typing_schema_derivations"],
        9
    );
    assert!(
        report["backend_models"]["lean"]["typing_schema_derivations"]
            .as_u64()
            .unwrap()
            < EXPECTED_KERNEL_RULE_COUNT as u64
    );
    let rules = report["rules"].as_array().unwrap();
    assert_eq!(rules.len(), exporter.rules().len());
    let smt = rules.iter().find(|r| r["rule_name"] == "K_Smt").unwrap();
    assert_eq!(smt["status"], "Unsupported");
    assert!(!smt["reason"].as_str().unwrap().is_empty());
    assert!(smt["framework_reference"].is_null());
    let var = rules.iter().find(|r| r["rule_name"] == "K_Var").unwrap();
    assert_eq!(var["status"], "Proved");
    assert!(var["reason"].is_null());
    let pi = rules
        .iter()
        .find(|r| r["rule_name"] == "K_Pi_Form")
        .unwrap();
    assert_eq!(pi["status"], "UnverifiedFrameworkReference");
    assert!(pi["framework_reference"]["citation"].is_string());
}
#[test]
fn cross_exports_include_only_supported_schema_lemmas() {
    let root = project();
    audit(root.path(), &["--kernel-soundness"]);
    let directory = root.path().join("target/audit-reports/kernel-soundness");
    let coq = fs::read_to_string(directory.join("kernel_soundness.v")).unwrap();
    let lean = fs::read_to_string(directory.join("KernelSoundness.lean")).unwrap();
    assert!(coq.contains("Theorem kernel_soundness"));
    assert!(lean.contains("theorem kernel_soundness"));
    for rule in SoundnessExporter::new().rules() {
        assert_eq!(
            coq.contains(&format!("Lemma {} :", rule.lemma_name)),
            rule.status.is_supported(),
            "{}",
            rule.rule_name
        );
        assert_eq!(
            lean.contains(&format!("theorem {} :", rule.lemma_name)),
            rule.status.is_supported(),
            "{}",
            rule.rule_name
        );
    }
    assert!(!coq.contains("| t_smt"));
    assert!(!lean.contains("| t_smt"));
    assert!(!coq.lines().any(|line| line.trim() == "Admitted."));
    assert!(
        !lean
            .lines()
            .any(|line| line.contains(":= sorry") || line.trim() == "sorry")
    );
}
#[test]
fn iou_dashboard_preserves_unsupported_partition() {
    let root = project();
    let report: serde_json::Value = serde_json::from_str(&audit(
        root.path(),
        &["--soundness-iou", "--format", "json"],
    ))
    .unwrap();
    assert_eq!(report["schema_version"], 3);
    assert_eq!(
        report["total_unsupported"],
        SoundnessExporter::new().unsupported_count()
    );
    assert_eq!(report["implementation_correspondence"], "not_established");
    let entries = report["categories"]["SmtAxiom"]["rules"]
        .as_array()
        .unwrap();
    assert!(
        entries
            .iter()
            .any(|r| r["rule_name"] == "K_Smt" && r["status"] == "Unsupported")
    );
    let plain = audit(root.path(), &["--soundness-iou"]);
    assert!(plain.contains("unsupported"));
    assert!(plain.contains("does not prove Rust checker correspondence"));
    assert!(!plain.contains("Every kernel rule is structurally proved"));
}

#[test]
fn trust_report_does_not_turn_references_into_discharges() {
    let root = project();
    let payload: serde_json::Value = serde_json::from_str(&audit(
        root.path(),
        &["--trust-extension-report", "--format", "json"],
    ))
    .unwrap();
    assert_eq!(payload["schema_version"], 2);
    let report = &payload["report"];
    let exporter = SoundnessExporter::new();
    assert_eq!(
        report["unverified_framework_reference_count"],
        exporter.unverified_framework_reference_count()
    );
    assert_eq!(report["unsupported_count"], exporter.unsupported_count());
    assert!(report.get("discharged_by_framework_count").is_none());
    let plain = audit(root.path(), &["--trust-extension-report"]);
    assert!(plain.contains("Framework references are unresolved obligations, not proof receipts"));
    assert!(!plain.contains("endgame"));
    assert!(!plain.contains("no open IOUs"));
}

#[test]
fn external_replay_refuses_unknown_backend_and_failed_fresh_export() {
    for backend in ["not-a-verifier", "agda"] {
        let root = project();
        let output = Command::new(env!("CARGO_BIN_EXE_verum"))
            .args([
                "audit",
                "--external-prover-replay",
                "--strict",
                "--backend",
                backend,
            ])
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported replay backend"));
    }
    let root = project();
    fs::create_dir(root.path().join("target")).unwrap();
    fs::write(
        root.path().join("target/audit-reports"),
        b"cannot create fresh exports here",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(["audit", "--external-prover-replay", "--backend", "isabelle"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(!output.status.success(), "failed export was ignored");
}
