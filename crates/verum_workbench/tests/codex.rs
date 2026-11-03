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
   printf '%s\n' '{"id":"review","method":"item/tool/call","params":{"threadId":"fixture","tool":"verum_propose","arguments":{}}}'
   printf '%s\n' '{"method":"serverRequest/resolved","params":{"threadId":"fixture","requestId":"review"}}'
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
    assert_eq!(events.read(0, Some("fixture"))["events"].as_array().unwrap().len(), 2);
    assert!(
        events.read(0, Some("fixture"))["pendingReviews"] == json!([]),
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

#[tokio::test]
async fn abandoning_a_partial_frame_invalidates_the_transport() {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("provider");
    std::fs::write(&executable, "#!/bin/sh\nread line\nprintf '%s\\n' '{\"id\":1,\"result\":{}}'\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let provider = Codex::connect_with_storage(&executable, dir.path(), dir.path()).await.unwrap();
    {
        // The provider retains stdin but does not drain it. This cannot fit in
        // a pipe, so cancellation interrupts a real partial NDJSON frame.
        let sending = provider.send(json!({"method":"fixture","payload":"x".repeat(2 * 1024 * 1024)}));
        tokio::pin!(sending);
        assert!(tokio::time::timeout(Duration::from_millis(30), &mut sending).await.is_err());
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while provider.connected() { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(provider.request("account/read", json!({})).await.is_err());
}
