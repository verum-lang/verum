use super::*;
use crate::{Config, Studio};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

async fn fixture() -> (tempfile::TempDir, Arc<Studio>) {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("provider");
    fs::write(&executable, r#"#!/bin/sh
while IFS= read -r line; do
    id=$(printf '%s' "$line" | sed -n 's/^{"id":\([0-9][0-9]*\),.*/\1/p')
    case "$line" in
        *'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$id" ;;
        *'"method":"thread/start"'*) printf '{"id":%s,"result":{"thread":{"id":"author"}}}\n' "$id" ;;
        *'"method":"turn/start"'*)
            printf '%s\n' '{"method":"turn/started","params":{"threadId":"author","turn":{"id":"A"}}}'
            printf '{"id":%s,"result":{"turn":{"id":"A"}}}\n' "$id"
            printf '%s\n' '{"id":"run","method":"item/tool/call","params":{"threadId":"author","turnId":"A","tool":"verum_run","arguments":{"path":"main.vr","source":"print(1);","timeout_ms":30000}}}'
            printf '%s\n' '{"id":"write","method":"item/tool/call","params":{"threadId":"author","turnId":"A","tool":"verum_propose","arguments":{"path":"new.vr","source":"fn example() {}","expected_hash":null}}}' ;;
        *'"method":"turn/interrupt"'*)
            printf '%s\n' '{"id":"late","method":"item/tool/call","params":{"threadId":"author","turnId":"A","tool":"verum_propose","arguments":{"path":"late.vr","source":"fn example() {}","expected_hash":null}}}'
            printf '{"id":%s,"result":{}}\n' "$id" ;;
        *'"method":"account/read"'*)
            case "$line" in
                *'"action":"disconnect"'*) exit 0 ;;
                *'"action":"complete"'*) printf '%s\n' '{"method":"turn/completed","params":{"threadId":"author","turn":{"id":"A"}}}' ;;
            esac
            printf '{"id":%s,"result":{}}\n' "$id" ;;
    esac
done
"#).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let worker = root.path().join("worker");
    fs::write(&worker, "#!/bin/sh\nsleep 30 &\nwait\n").unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(root.path().join("main.vr"), "print(1);").unwrap();
    let studio = Arc::new(
        Studio::open(Config {
            workspace: root.path().into(),
            codex: executable.clone(),
            verum: worker.clone(),
            proof: worker,
            port: 0,
        })
        .unwrap(),
    );
    let provider = crate::codex::Codex::connect_with_storage(&executable, root.path(), root.path())
        .await
        .unwrap();
    *studio.agent.lock().await = Some(Arc::new(provider));
    studio
        .call(
            "agent.request",
            json!({"method":"thread/start","params":{}}),
        )
        .await
        .unwrap();
    studio
        .call(
            "agent.request",
            json!({"method":"turn/start","params":{"threadId":"author","input":[]}}),
        )
        .await
        .unwrap();
    (root, studio)
}

async fn approval(studio: &Studio, wire: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let events = studio.call("agent.events", json!({"threadId":"author"})).await.unwrap();
            if let Some(entry) = events["events"].as_array().unwrap().iter().find(|entry| entry["message"]["id"] == wire) {
                return json!({"reviewId":entry["reviewId"],"threadId":"author","result":{"approved":true}});
            }
            tokio::task::yield_now().await;
        }
    }).await.unwrap()
}

async fn end(studio: &Studio, event: &str) {
    let request = if event == "interrupt" {
        json!({"method":"turn/interrupt","params":{"threadId":"author","turnId":"A"}})
    } else {
        json!({"method":"account/read","params":{"action":event}})
    };
    let result = studio.call("agent.request", request).await;
    if event != "disconnect" {
        result.unwrap();
    }
}

async fn receipts(studio: &Studio, predicate: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let rows = studio.call("operation.list", json!({})).await.unwrap();
            if predicate(&rows) {
                return rows;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("owned process must settle without waiting for its execution timeout")
}

#[tokio::test]
async fn turn_termination_cancels_owned_execution_but_not_user_jobs() {
    for event in ["interrupt", "complete", "disconnect"] {
        let (root, studio) = fixture().await;
        let run = approval(&studio, "run").await;
        let write = approval(&studio, "write").await;
        let responder = {
            let studio = studio.clone();
            tokio::spawn(async move { studio.call("agent.respond", run).await })
        };
        let started = receipts(&studio, |rows| rows.as_array().unwrap().len() == 1).await;
        assert_eq!(started[0]["agent"]["conversation"], "author");
        assert_eq!(started[0]["agent"]["turn"], "A");
        let independent = studio
            .call(
                "operation.start",
                json!({"kind":"run","path":"main.vr","source":"print(2);","timeout_ms":30000}),
            )
            .await
            .unwrap();
        end(&studio, event).await;
        let settled = receipts(&studio, |rows| rows[0]["status"] != "running").await;
        assert_eq!(settled[0]["status"], "cancelled", "{event}: {settled}");
        assert_eq!(settled[1]["status"], "running");
        assert!(settled[1].get("agent").is_none());
        assert_eq!(responder.await.unwrap().unwrap()["delivered"], false);
        assert!(studio.call("agent.respond", write).await.is_err());
        assert!(!root.path().join("new.vr").exists());
        studio.call("operation.cancel", independent).await.unwrap();
        receipts(&studio, |rows| rows[1]["status"] == "cancelled").await;
    }
}

#[tokio::test]
async fn cancelled_approval_waiting_for_workspace_cannot_write_or_spawn() {
    for wire in ["run", "write"] {
        let (root, studio) = fixture().await;
        let review = approval(&studio, wire).await;
        let locked = studio.workspace.lock().await;
        let responder = {
            let studio = studio.clone();
            tokio::spawn(async move { studio.call("agent.respond", review).await })
        };
        // Wait until approval is consumed and the operation is waiting for the
        // resource. Cancellation must win before synchronous mutation admission.
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let events = studio
                    .call("agent.events", json!({"threadId":"author"}))
                    .await
                    .unwrap();
                if events["pendingReviews"].as_array().unwrap().len() == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        end(&studio, "interrupt").await;
        drop(locked);
        assert_eq!(responder.await.unwrap().unwrap()["delivered"], false);
        assert_eq!(
            studio.call("operation.list", json!({})).await.unwrap(),
            json!([])
        );
        assert!(!root.path().join("new.vr").exists());
    }
}

#[tokio::test]
async fn dropping_responder_cancels_its_child_process() {
    let (_root, studio) = fixture().await;
    let review = approval(&studio, "run").await;
    let responder = {
        let studio = studio.clone();
        tokio::spawn(async move { studio.call("agent.respond", review).await })
    };
    receipts(&studio, |rows| rows.as_array().unwrap().len() == 1).await;
    responder.abort();
    assert!(responder.await.unwrap_err().is_cancelled());
    let rows = receipts(&studio, |rows| rows[0]["status"] != "running").await;
    assert_eq!(rows[0]["status"], "cancelled");
    assert!(
        rows[0]["termination"]
            .as_str()
            .unwrap()
            .contains("abandoned")
    );
}
