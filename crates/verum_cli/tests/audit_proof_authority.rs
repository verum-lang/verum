//! CLI audit reports must not certify metadata as mathematical proofs (T1774).
use std::{fs, path::PathBuf, process::Command};

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str, source: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "verum-audit-authority-{name}-{}",
            std::process::id()
        ));
        fs::create_dir_all(path.join("src")).unwrap();
        fs::write(
            path.join("verum.toml"),
            "[package]\nname=\"authority-test\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        fs::write(path.join("src/lib.vr"), source).unwrap();
        Self(path)
    }
    fn audit(&self, flag: &str, report: &str) -> serde_json::Value {
        let output = Command::new(env!("CARGO_BIN_EXE_verum"))
            .args(["audit", flag, "--format", "json"])
            .current_dir(&self.0)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "metadata-only proof gate succeeded: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let bytes =
            fs::read(self.0.join("target/audit-reports").join(report)).unwrap_or_else(|e| {
                panic!(
                    "missing report: {e}; stderr: {}",
                    String::from_utf8_lossy(&output.stderr)
                )
            });
        serde_json::from_slice(&bytes).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn known_intrinsic_annotation_is_not_a_theorem_discharge() {
    let scratch = Scratch::new(
        "intrinsic",
        "@kernel_discharge(\"kernel_soundness_v0\") public axiom claimed() ensures Bool;",
    );
    let report = scratch.audit(
        "--kernel-discharged-axioms",
        "kernel-discharged-axioms.json",
    );
    assert_eq!(report["proof_status"], "structural_only");
    assert_eq!(report["discharge_count"], 0);
    assert!(report["citation_count"].as_u64().unwrap() > 0);
    let own = report["discharges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["axiom"] == "claimed")
        .unwrap();
    assert_eq!(own["recognised"], true);
}
#[test]
fn all_positive_reflection_samples_still_require_proofs() {
    let scratch = Scratch::new("reflection", "");
    let report = scratch.audit("--reflection-tower", "reflection-tower.json");
    assert_eq!(report["proof_status"], "structural_only");
    assert_eq!(report["load_bearing"], false);
    assert_eq!(report["structurally_consistent"], true);
    assert_eq!(report["discharged_stage_count"], 0);
    assert_eq!(report["constructive_discharged_count"], 0);
    for stage in report["stages"].as_array().unwrap() {
        assert_eq!(stage["discharges"], false);
        assert_eq!(stage["diagnostic_holds"], true);
    }
}

#[test]
fn resolved_kernel_prefix_is_not_l4_proof_authority() {
    let scratch = Scratch::new(
        "apply",
        r#"
public axiom kernel_fiction_strict() ensures Bool;
public theorem claimed() ensures Bool proof { apply kernel_fiction_strict(); }
"#,
    );
    let report = scratch.audit("--apply-graph", "apply-graph.json");
    assert_eq!(report["proof_status"], "structural_only");
    assert_eq!(report["load_bearing"], false);
    let own = report["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["theorem"] == "claimed")
        .unwrap();
    assert_eq!(own["dependencies_resolved"], true);
    assert_eq!(own["kernel_strict"], 1);
    assert_eq!(own["load_bearing"], false);
    assert_eq!(own["proof_status"], "structural_only");
}
