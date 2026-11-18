use crate::{Config, Studio};
use serde_json::{Value, json};

#[tokio::test]
async fn discovery_does_not_connect_or_claim_account_access() {
    let root = tempfile::tempdir().unwrap();
    let executable = std::env::current_exe().unwrap();
    let studio = Studio::open_with_providers(
        Config {
            workspace: root.path().into(),
            codex: executable,
            verum: "unused".into(),
            proof: "unused".into(),
            port: 0,
        },
        None,
        root.path().join("missing"),
    )
    .unwrap();
    let catalog = studio.call("agent.providers", json!({})).await.unwrap();
    let drivers = catalog["providers"].as_array().unwrap();
    let codex = drivers.iter().find(|p| p["id"] == "codex").unwrap();
    let agy = drivers.iter().find(|p| p["id"] == "agy").unwrap();
    assert_eq!(codex["executableAvailable"], true);
    assert_eq!(codex["capabilities"]["reviewedTools"], true);
    assert_eq!(agy["executableAvailable"], false);
    assert_eq!(agy["capabilities"]["workspaceContext"], false);
    for driver in drivers {
        assert_eq!(driver["connectionId"], Value::Null);
        assert!(driver.get("authenticated").is_none());
    }
    assert!(
        studio
            .call("agent.connect", json!({"provider":"unknown-driver"}))
            .await
            .is_err()
    );
}
