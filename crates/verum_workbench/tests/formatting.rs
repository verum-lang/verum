#![cfg(unix)]
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use verum_workbench::{Config, Studio};

#[tokio::test]
#[ignore = "requires VERUM_TEST_FORMATTER built from this revision"]
async fn actual_cli_formats_through_the_studio_worker_without_losing_source() {
    let root = tempfile::tempdir().unwrap();
    let library = std::env::var_os("VERUM_TEST_LIBRARY_METADATA").map(|path| {
        verum_lsp::library_metadata::LibraryMetadata::load(std::path::Path::new(&path)).unwrap()
    });
    let studio = Studio::open_with_library(
        Config {
            workspace: root.path().into(),
            verum: std::env::var_os("VERUM_TEST_FORMATTER").unwrap().into(),
            proof: "unused".into(),
            codex: "unused".into(),
            port: 0,
        },
        library,
    )
    .unwrap();
    let source = "// keep Ω😀\nfn main()->Int{42}\n";
    let result = studio
        .call(
            "language.request",
            json!({"path":"main.vr", "source":source,
        "method":"textDocument/formatting", "options":{"tabSize":4,"insertSpaces":true}}),
        )
        .await
        .unwrap();
    assert_eq!(
        result["result"][0]["newText"],
        "// keep Ω😀\nfn main() -> Int {\n    42\n}\n"
    );
    assert!(
        !root.path().join("main.vr").exists(),
        "formatting must not save a draft"
    );
}

#[tokio::test]
async fn unavailable_formatter_is_an_error_and_never_substitutes_lossy_edits() {
    let root = tempfile::tempdir().unwrap();
    let source = "// keep this comment\nfn main() -> Int { 42 }\n";
    std::fs::write(root.path().join("main.vr"), source).unwrap();
    let studio = Studio::open(Config {
        workspace: root.path().into(),
        verum: root.path().join("missing-formatter"),
        proof: "unused".into(),
        codex: "unused".into(),
        port: 0,
    })
    .unwrap();
    let error = studio.call("language.request", json!({"path":"main.vr", "method":"textDocument/formatting", "options":{"tabSize":4,"insertSpaces":true}})).await.unwrap_err();
    assert!(error.to_string().contains("Start formatter"), "{error:#}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("main.vr")).unwrap(),
        source
    );
}

#[tokio::test]
async fn formatting_uses_the_configured_cli_and_returns_bound_edits_without_saving() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("formatter");
    // Require the CLI protocol, and consume all input including EOF. A child
    // that is not given EOF would hang rather than pass this integration test.
    std::fs::write(&binary, "#!/bin/sh\n[ \"$1\" = fmt ] && [ \"$2\" = --stdin ] || exit 1\ncat >/dev/null\nprintf 'fn main() -> Int { 42 }\\n'\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let saved = "fn saved() {}\n";
    std::fs::write(root.path().join("main.vr"), saved).unwrap();
    let studio = Studio::open(Config {
        workspace: root.path().into(),
        verum: binary,
        proof: "verum-source-replay".into(),
        codex: "codex".into(),
        port: 0,
    })
    .unwrap();
    let source = "fn main()->Int{42}";
    let request = json!({"path":"main.vr","source":source,"method":"textDocument/formatting","options":{"tabSize":4,"insertSpaces":true}});
    let formatted = studio
        .call("language.request", request.clone())
        .await
        .unwrap();
    assert_eq!(
        formatted["source_hash"],
        verum_workbench::workspace::digest(source.as_bytes()).as_str()
    );
    assert_eq!(
        formatted["result"][0]["newText"],
        "fn main() -> Int { 42 }\n"
    );
    assert_eq!(
        formatted["result"][0]["range"]["end"],
        json!({"line":0,"character":source.len()})
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("main.vr")).unwrap(),
        saved
    );
    let mut invalid = request;
    invalid["options"]["tabSize"] = json!(0);
    assert!(
        studio
            .call("language.request", invalid)
            .await
            .unwrap_err()
            .to_string()
            .contains("Tab size")
    );
}
