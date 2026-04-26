//! Smoke test for `verum_lsp::lint_diagnostics::lint_diagnostics`.
//!
//! Builds a temp fixture project that triggers a lint rule, asks
//! the function to lint the fixture's main.vr, and asserts at
//! least one Diagnostic comes back.

use std::path::PathBuf;

use tower_lsp::lsp_types::Url;
use verum_lsp::lint_diagnostics::{lint_diagnostics, LintSettings};

fn binary_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p.push("release");
    p.push("verum");
    p
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
