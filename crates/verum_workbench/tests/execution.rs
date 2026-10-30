use serde_json::json;
use verum_workbench::{Config, Studio};

#[tokio::test]
async fn shared_planning_is_read_only_and_uses_the_workspace_boundary() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("main.vr"), "saved source").unwrap();
    let studio = Studio::open(Config {
        workspace: root.path().into(),
        verum: "verum".into(),
        proof: "verum-source-replay".into(),
        codex: "codex".into(),
        port: 0,
    })
    .unwrap();
    let request = json!({"kind":"run","path":"main.vr","source":"print(42);","permissions":"deny-all","timeout_ms":5000});
    let plan = studio
        .call("operation.plan", request.clone())
        .await
        .unwrap();
    assert_eq!(
        plan["source_hash"],
        json!(verum_workbench::workspace::digest(b"print(42);"))
    );
    assert_eq!(plan["permissions"], "deny-all");
    assert_eq!(plan["timeout_ms"], 5000);
    assert_eq!(
        studio.call("operation.list", json!({})).await.unwrap(),
        json!([])
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("main.vr")).unwrap(),
        "saved source"
    );
    assert_eq!(
        std::fs::read_dir(root.path().join(".verum-studio/runs"))
            .unwrap()
            .count(),
        0
    );
    for path in ["../escape.vr", "/escape.vr", ".private.vr"] {
        let mut invalid = request.clone();
        invalid["path"] = json!(path);
        assert!(studio.call("operation.plan", invalid).await.is_err());
    }
    let tools = verum_workbench::codex::tools();
    let schema = |name| {
        tools
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap()["inputSchema"]
            .clone()
    };
    let planning = schema("verum_plan");
    let running = schema("verum_run");
    for field in ["permissions", "timeout_ms", "source", "path"] {
        assert_eq!(planning["properties"][field], running["properties"][field]);
    }
    assert_eq!(planning["additionalProperties"], false);
    assert_eq!(
        schema("verum_check")["properties"]["permissions"],
        json!(null)
    );
    assert_eq!(
        schema("verum_proof")["properties"]["permissions"],
        json!(null)
    );
}
