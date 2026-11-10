//! Smoke test for `verum_lsp::lint_diagnostics::lint_diagnostics`.
//!
//! Builds a temp fixture project that triggers a lint rule, asks
//! the function to lint the fixture's main.vr, and asserts at
//! least one Diagnostic comes back.

use std::path::PathBuf;

use tower_lsp::lsp_types::Url;
use verum_lsp::lint_diagnostics::{LintSettings, lint_diagnostics};

fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

#[tokio::test]
async fn studio_and_cli_inspect_the_same_saved_sources_and_machine_edits() {
    use verum_workbench::{Config, Studio};
    let fixture = make_fixture();
    let root = fixture.path();
    std::fs::create_dir_all(root.join("packages/nested")).unwrap();
    std::fs::write(
        root.join("packages/nested/verum.toml"),
        "[package]\nname='nested'\n",
    )
    .unwrap();
    std::fs::write(root.join("packages/nested/lib.vr"), "fn inspect() {}\n").unwrap();
    let binary = binary_path();
    let cli = std::process::Command::new(&binary)
        .args(["lint", "--inspect", "--threads", "0"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&cli.stdout).unwrap();
    let studio = Studio::open(Config {
        workspace: root.into(),
        verum: binary,
        proof: "unused".into(),
        codex: "unused".into(),
        port: 0,
    })
    .unwrap();
    let report = studio
        .call("workspace.lint", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(expected, report);
    assert_eq!(report["files"].as_array().unwrap().len(), 2);
    assert!(
        report["files"][1]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["rule"] == "deprecated-syntax")
    );
}

fn make_fixture() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("verum_lsp_lint_smoke_")
        .tempdir()
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("src dir");
    std::fs::write(
        dir.path().join("verum.toml"),
        "[package]\nname = \"smoke\"\nversion = \"0.1.0\"\n",
    )
    .expect("manifest");
    std::fs::write(
        dir.path().join("src").join("main.vr"),
        "fn main() {\n    let x = Box::new(5);\n}\n",
    )
    .expect("main.vr");
    dir
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lint_diagnostics_returns_diagnostic_for_deprecated_syntax() {
    let dir = make_fixture();
    let path = dir.path().join("src").join("main.vr");
    let uri = Url::from_file_path(&path).expect("file URL");

    let settings = LintSettings {
        enabled: true,
        profile: None,
        binary: Some(binary_path()),
    };
    let diagnostics = lint_diagnostics(&uri, &settings).await;
    assert!(
        diagnostics.iter().any(|d| {
            matches!(&d.code, Some(tower_lsp::lsp_types::NumberOrString::String(s)) if s == "deprecated-syntax")
        }),
        "expected deprecated-syntax diagnostic, got: {:?}",
        diagnostics.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lint_diagnostics_returns_empty_when_disabled() {
    let dir = make_fixture();
    let path = dir.path().join("src").join("main.vr");
    let uri = Url::from_file_path(&path).expect("file URL");

    let settings = LintSettings {
        enabled: false,
        profile: None,
        binary: Some(binary_path()),
    };
    let diagnostics = lint_diagnostics(&uri, &settings).await;
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics when disabled, got: {:?}",
        diagnostics.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lint_diagnostics_silent_when_no_project_root() {
    // A file with no surrounding verum.toml should produce no
    // diagnostics — the lint engine has no project to lint against.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("loose.vr");
    std::fs::write(&path, "fn main() {}\n").expect("loose file");
    let uri = Url::from_file_path(&path).expect("file URL");

    let settings = LintSettings {
        enabled: true,
        profile: None,
        binary: Some(binary_path()),
    };
    let diagnostics = lint_diagnostics(&uri, &settings).await;
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics for orphan file, got: {:?}",
        diagnostics.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
}
