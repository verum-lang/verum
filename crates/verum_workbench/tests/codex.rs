#![cfg(unix)]
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, time::Duration};
use verum_workbench::codex::Codex;

#[tokio::test]
async fn transport_routes_interleaved_approvals_and_expires_resolved_requests() {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("provider");
    std::fs::write(&executable,r##"#!/bin/sh
while IFS= read -r line; do
 case "$line" in
  *'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{"userAgent":"fixture"}}' ;;
  *'"method":"account/read"'*)
   printf '%s\n' '{"id":"review","method":"item/tool/call","params":{"tool":"verum_propose","arguments":{}}}'
   printf '%s\n' '{"method":"serverRequest/resolved","params":{"requestId":"review"}}'
   printf '%s\n' '{"id":2,"result":{"account":null}}' ;;
 esac
done
"##).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let provider = Codex::connect_with_storage(&executable, dir.path(), dir.path())
        .await
        .unwrap();
    assert_eq!(
        provider.request("account/read", json!({})).await.unwrap(),
        json!({"account":null})
    );
    let events = provider.events.lock().await;
    assert_eq!(events.read(0)["events"].as_array().unwrap().len(), 2);
    assert!(
        events.requests.is_empty(),
        "Resolved approval must not authorize a later source write"
    );
}
#[tokio::test]
async fn disconnected_provider_fails_pending_requests_without_waiting_for_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("provider");
    std::fs::write(&executable,"#!/bin/sh\nread line\nprintf '%s\\n' '{\"id\":1,\"result\":{}}'\nread line\nread line\nexit 0\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let provider = Codex::connect_with_storage(&executable, dir.path(), dir.path())
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        provider.request("account/read", json!({})),
    )
    .await
    .unwrap();
    assert!(result.is_err());
}
