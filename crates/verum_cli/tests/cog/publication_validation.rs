//! T1636: the dispatched publication handler must not overstate dry-run validity.
//! Only key discovery is supplied explicitly; archive creation, Ed25519 signing,
//! metadata construction and the public transport validator run normally.

use super::publish_with_signing_keys;
use std::process::Command;
use tempfile::TempDir;

const CHILD: &str = "cog::publication_validation::isolated_publication_case";
const CASE_ENV: &str = "VERUM_T1636_PUBLICATION_CASE";

#[test]
fn unsigned_dry_run_passes_wire_admission() {
    child_case("unsigned");
}

#[test]
fn signed_dry_run_refuses_unsupported_evidence_without_success_claim() {
    child_case("signed");
}

fn child_case(case: &str) {
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture"])
        .env(CASE_ENV, case)
        .output()
        .unwrap();
    let output = format!(
        "{}\n{}",
        std::str::from_utf8(&result.stdout).unwrap(),
        std::str::from_utf8(&result.stderr).unwrap()
    );
    assert!(result.status.success(), "handler case {case}: {output}");
    assert_eq!(
        output.contains("valid for publishing"),
        case == "unsigned",
        "{output}"
    );
    if case == "signed" {
        assert!(
            output.contains("Cog signed with Ed25519"),
            "the real signing path must run: {output}"
        );
    }
}

#[test]
fn isolated_publication_case() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let project = TempDir::new().unwrap();
    std::env::set_current_dir(project.path()).unwrap();
    let name = format!("publication-fixture-{}", std::process::id());
    std::fs::write(project.path().join("Verum.toml"), format!("[cog]\nname = \"{name}\"\nversion = \"1.2.3\"\n[registry]\nindex = \"http://127.0.0.1:1\"\n")).unwrap();
    if case == "signed" {
        let key = project.path().join("fixture-signing.key");
        std::fs::write(&key, [7_u8; 32]).unwrap();
        let error = publish_with_signing_keys(true, true, &[key]).unwrap_err();
        assert!(
            error.to_string().contains("source metadata only"),
            "{error}"
        );
    } else {
        publish_with_signing_keys(true, true, &[]).unwrap();
    }
    assert!(
        !std::env::temp_dir()
            .join(format!("{name}-1.2.3.vr"))
            .exists(),
        "dry-run archive must be cleaned up"
    );
}
