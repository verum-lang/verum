use serde_json::{Value, json};
use verum_workbench::{Config, Studio};

fn studio(root: &std::path::Path) -> Studio {
    Studio::open(Config {
        workspace: root.to_path_buf(),
        verum: "verum".into(),
        proof: "verum-source-replay".into(),
        codex: "codex".into(),
        port: 0,
    })
    .unwrap()
}

#[tokio::test]
async fn actual_lsp_session_shares_revision_diagnostics_architecture_and_navigation() {
    let root = tempfile::tempdir().unwrap();
    let studio = studio(root.path());
    let source = "type Point is { x: Int, y: Int };\nfn identity(x: Int) -> Int { x }\nfn main() { let value = identity(42); }\n";
    let args = json!({"path":"main.vr", "source":source});
    let analysis = studio.call("language.request", args.clone()).await.unwrap();
    assert_eq!(analysis["result"]["parsed"], true);
    assert_eq!(analysis["result"]["source_hash"], analysis["source_hash"]);
    assert_eq!(
        analysis["result"]["declarations"].as_array().unwrap().len(),
        3
    );
    assert_eq!(analysis["result"]["diagnostics"], json!([]));
    let mut request = args.clone();
    request["method"] = json!("textDocument/definition");
    request["position"] = json!({"line":2,"character":28});
    let definition = studio.call("language.request", request).await.unwrap();
    assert_eq!(definition["source_hash"], analysis["source_hash"]);
    assert!(!definition["result"].is_null(), "{definition}");
    let error = studio
        .call(
            "language.request",
            json!({"path":"main.vr","source":"fn main() { let value: Int = true; }"}),
        )
        .await
        .unwrap();
    assert!(error["version"].as_i64().unwrap() > analysis["version"].as_i64().unwrap());
    assert!(
        !error["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let restored = studio.call("language.request", args).await.unwrap();
    assert_eq!(restored["source_hash"], analysis["source_hash"]);
    assert!(restored["version"].as_i64().unwrap() > error["version"].as_i64().unwrap());
    assert_eq!(restored["result"]["diagnostics"], json!([]));
    let project = studio.call("project.inspect", json!({})).await.unwrap();
    assert_eq!(
        project["analysis"]["coverage"],
        "open-documents-and-source-dependencies"
    );
    assert_eq!(
        project["analysis"]["documents"][0]["source_hash"],
        restored["source_hash"]
    );
    assert_eq!(
        project["analysis"]["documents"][0]["version"],
        restored["version"]
    );
}

#[tokio::test]
async fn context_requirements_are_projected_from_the_same_ast_and_exact_source_ranges() {
    let root = tempfile::tempdir().unwrap();
    let studio = studio(root.path());
    let source = "context Logger { fn log(message: Text); }\nfn report(value: Int) using [Logger] { print(value); }";
    let result = studio
        .call(
            "language.request",
            json!({"path":"contexts.vr","source":source}),
        )
        .await
        .unwrap();
    assert_eq!(result["result"]["parsed"], true, "{result}");
    let declarations = result["result"]["declarations"].as_array().unwrap();
    assert_eq!(declarations[0]["kind"], "context");
    assert_eq!(declarations[1]["requirements"][0]["source"], "Logger");
    assert_eq!(
        declarations[1]["requirements"][0]["range"]["start"]["line"],
        1
    );
}

#[tokio::test]
async fn saved_source_analysis_and_submitted_drafts_share_one_service_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let saved = "fn saved() -> Int { 7 }";
    std::fs::write(root.path().join("saved.vr"), saved).unwrap();
    let studio = studio(root.path());
    let from_disk = studio
        .call("language.request", json!({"path":"saved.vr"}))
        .await
        .unwrap();
    let draft = studio
        .call(
            "language.request",
            json!({"path":"saved.vr","source":"fn draft() -> Int { 9 }"}),
        )
        .await
        .unwrap();
    assert_ne!(from_disk["source_hash"], draft["source_hash"]);
    assert_eq!(
        std::fs::read_to_string(root.path().join("saved.vr")).unwrap(),
        saved
    );
    for path in ["../escape.vr", "/escape.vr", ".hidden.vr"] {
        assert!(
            studio
                .call("language.request", json!({"path":path,"source":saved}))
                .await
                .is_err()
        );
    }
    assert!(
        studio
            .call(
                "language.request",
                json!({"path":"saved.vr","method":"workspace/executeCommand","source":saved})
            )
            .await
            .is_err()
    );
    let tools = verum_workbench::codex::tools();
    assert!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "verum_analyze")
    );
    let _: Value = from_disk;
}

#[tokio::test]
async fn project_dependency_edits_reach_the_same_studio_and_agent_snapshot() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("Verum.toml"),
        "[cog]\nname='workspace-demo'\n",
    )
    .unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    let dependency = "public fn answer() -> Int { 42 }";
    std::fs::write(root.path().join("src/dependency.vr"), dependency).unwrap();
    let studio = studio(root.path());
    let args = json!({"path":"src/main.vr","source":"mount workspace_demo.dependency.{answer}; fn main() -> Int { answer() }"});
    let first = studio.call("language.request", args.clone()).await.unwrap();
    assert_eq!(first["result"]["diagnostics"], json!([]), "{first:#}");
    assert_eq!(first["result"]["bindings"][0]["name"], "answer");
    studio
        .call(
            "language.request",
            json!({"path":"src/dependency.vr","source":"public fn answer() -> Bool { true }"}),
        )
        .await
        .unwrap();
    let after = studio.call("language.request", args).await.unwrap();
    assert_eq!(after["version"], first["version"]);
    assert_ne!(
        after["result"]["project_revision"],
        first["result"]["project_revision"]
    );
    assert!(
        !after["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let project = studio.call("project.inspect", json!({})).await.unwrap();
    assert_eq!(
        project["analysis"]["project_revision"],
        after["result"]["project_revision"]
    );
    assert_eq!(project["analysis"]["sources"].as_array().unwrap().len(), 2);
    assert_eq!(
        std::fs::read_to_string(root.path().join("src/dependency.vr")).unwrap(),
        dependency
    );
}
