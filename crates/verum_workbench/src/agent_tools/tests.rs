use super::*;
use crate::{Config, Studio};

#[test]
fn only_host_registered_routes_receive_review_categories() {
    assert_eq!(review_kind("verum_propose"), Some("source"));
    for tool in ["verum_run", "verum_check", "verum_proof"] {
        assert_eq!(review_kind(tool), Some("execution"));
    }
    for tool in [
        "verum_format",
        "verum_analyze",
        "verum_plan",
        "verum_receipt",
    ] {
        assert_eq!(review_kind(tool), Some("read"));
    }
    assert_eq!(review_kind("run_shell"), None);
}

#[test]
fn an_analysis_approval_cannot_select_another_operation() {
    for args in [
        json!({"path":"main.vr","method":"project"}),
        json!({"path":"main.vr","method":"textDocument/formatting"}),
        json!({"path":"main.vr","method":"textDocument/definition"}),
        json!({"path":"main.vr","kind":"run"}),
    ] {
        assert!(operation(&json!({"tool":"verum_analyze","arguments":args})).is_err());
    }
    assert!(
        operation(&json!({"tool":"verum_propose","arguments":{"path":"x.vr","source":"..."}}))
            .is_err()
    );
}

#[test]
fn saved_and_unsaved_analysis_keep_their_revision_and_default_operation() {
    for args in [
        json!({"path":"module.vr"}),
        json!({"path":"module.vr","source":"fn hello() {}"}),
    ] {
        let (method, routed) =
            operation(&json!({"tool":"verum_analyze","arguments":args})).unwrap();
        assert_eq!(method, "language.request");
        assert_eq!(routed, args);
    }
    for request in [
        json!({"tool":"verum_files","arguments":{"path":"sub","query":"cfg"}}),
        json!({"tool":"verum_changes","arguments":{"side":"index"}}),
    ] {
        assert!(operation(&request).is_err());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn receipt_lookup_preserves_the_existing_outcome_without_launching_a_job() {
    let root = tempfile::tempdir().unwrap();
    let source = "fn main() -> Int { 73 }";
    std::fs::write(root.path().join("main.vr"), source).unwrap();
    let studio = Studio::open(Config {
        workspace: root.path().into(),
        codex: "unused".into(),
        verum: "/usr/bin/false".into(),
        proof: "unused".into(),
        port: 0,
    })
    .unwrap();
    let job = studio
        .call(
            "operation.start",
            json!({"kind":"check","path":"main.vr","source":source}),
        )
        .await
        .unwrap();
    let request = json!({"tool":"verum_receipt","arguments":{"id":job["id"]}});
    let receipt = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let receipt = studio.tool(&request).await.unwrap();
            if receipt["status"] != "running" {
                return receipt;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(receipt["status"], "failed", "{receipt}");
    assert_eq!(receipt["proof"], Value::Null);
    for expected in [
        json!("invalid"),
        Value::Null,
        json!(crate::workspace::digest(b"fn main() -> Int { 72 + 1 }")),
    ] {
        let mut mismatch = request.clone();
        mismatch["arguments"]["expected_source_hash"] = expected;
        assert!(studio.tool(&mismatch).await.is_err());
    }
    let mut pinned = request.clone();
    pinned["arguments"]["expected_source_hash"] =
        json!(crate::workspace::digest(source.as_bytes()));
    assert_eq!(receipt, studio.tool(&pinned).await.unwrap());
    assert_eq!(receipt, studio.call("operation.read", job).await.unwrap());
    assert!(
        studio
            .tool(&json!({"tool":"verum_receipt","arguments":{"id":"missing"}}))
            .await
            .is_err()
    );
    assert_eq!(
        studio.call("operation.list", json!({})).await.unwrap(),
        json!([receipt])
    );
}

#[cfg(unix)]
#[tokio::test]
async fn registered_author_and_reviewer_share_navigation_but_have_distinct_authority() {
    use std::{fs, os::unix::fs::PermissionsExt, sync::Arc};
    let root = tempfile::tempdir().unwrap();
    let storage = tempfile::tempdir().unwrap();
    let executable = storage.path().join("provider");
    fs::write(&executable, r#"#!/bin/sh
started=0
while IFS= read -r line; do
    case "$line" in
        *'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}' ;;
        *'"method":"thread/start"'*)
            started=$((started + 1))
            printf '%s\n' "$line" > ".provider-start-$started"
            case "$started" in
                1) printf '%s\n' '{"id":2,"result":{"thread":{"id":"author"}}}' ;;
                2) printf '%s\n' '{"id":3,"result":{"thread":{"id":"critic"}}}'
                printf '%s\n' '{"id":"go","method":"item/tool/call","params":{"threadId":"critic","turnId":"B","tool":"verum_analyze","arguments":{"path":"main.vr","method":"textDocument/definition","position":{"line":1,"character":21}}}}' ;;
                3) printf '%s\n' '{"id":5,"result":{"thread":{"id":"author"}}}' ;;
            esac ;;
        *'"method":"account/read"'*)
            printf '%s\n' '{"id":"edit","method":"item/tool/call","params":{"threadId":"author","turnId":"A","tool":"verum_propose","arguments":{"path":"accepted.vr","source":"fn example() {}","expected_hash":null}}}'

            for thread in critic unknown; do
                printf '%s\n' '{"id":"edit-'$thread'","method":"item/tool/call","params":{"threadId":"'$thread'","turnId":"B","tool":"verum_propose","arguments":{"path":"forbidden.vr","source":"fn example() {}","expected_hash":null}}}'
            done
            printf '%s\n' '{"id":"run","method":"item/tool/call","params":{"threadId":"critic","turnId":"B","tool":"verum_run","arguments":{"path":"main.vr","source":"fn example() {}"}}}'
            printf '%s\n' '{"id":"stale","method":"item/tool/call","params":{"threadId":"author","turnId":"A","tool":"verum_propose","arguments":{"path":"forbidden.vr","source":"fn example() {}","expected_hash":null}}}'
            printf '%s\n' '{"id":4,"result":{}}' ;;
    esac
done
"#).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        root.path().join("main.vr"),
        "fn answer() -> Int { 41 }\nfn main() -> Int { answer() }\n",
    )
    .unwrap();
    let studio = Studio::open(Config {
        workspace: root.path().into(),
        codex: executable.clone(),
        verum: "unused".into(),
        proof: "unused".into(),
        port: 0,
    })
    .unwrap();
    let provider =
        crate::codex::Codex::connect_with_storage(&executable, root.path(), storage.path())
            .await
            .unwrap();
    *studio.agent.lock().await = Some(Arc::new(provider));
    let author = json!({"method":"thread/start","params":{}});
    let reviewer = json!({"method":"thread/start","role":"reviewer","params":{"role":"author","baseInstructions":"override","dynamicTools":[]}});
    studio.call("agent.request", author.clone()).await.unwrap();
    studio.call("agent.request", reviewer).await.unwrap();
    let sent = |i| {
        serde_json::from_slice::<Value>(
            &fs::read(root.path().join(format!(".provider-start-{i}"))).unwrap(),
        )
        .unwrap()["params"]
            .clone()
    };
    assert_eq!(
        sent(1)["dynamicTools"],
        studio.call("agent.tools", json!({})).await.unwrap()
    );
    assert_eq!(sent(2)["dynamicTools"], Role::Reviewer.catalog());
    assert!(sent(2).get("role").is_none());
    assert!(
        sent(2)["dynamicTools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "verum_receipt")
    );
    assert!(
        sent(2)["dynamicTools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| review_kind(t["name"].as_str().unwrap()) == Some("read"))
    );
    assert!(
        studio
            .call(
                "agent.request",
                json!({"method":"turn/start","params":{"threadId":"unknown","input":[]}})
            )
            .await
            .is_err()
    );
    studio
        .call(
            "agent.request",
            json!({"method":"account/read","params":{}}),
        )
        .await
        .unwrap();

    async fn review(studio: &Studio, thread: &str, wire: &str) -> Result<Value> {
        let events = studio
            .call("agent.events", json!({"after":0,"threadId":thread}))
            .await?;
        let id = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["message"]["id"] == wire)
            .context("missing fixture review")?["reviewId"]
            .clone();
        studio
            .call(
                "agent.respond",
                json!({"reviewId":id,"threadId":thread,"result":{"approved":true}}),
            )
            .await
    }
    let editor = studio.call("language.request",json!({"path":"main.vr","method":"textDocument/definition","position":{"line":1,"character":21}})).await.unwrap();
    assert!(!editor["result"].is_null());
    assert_eq!(
        review(&studio, "critic", "go").await.unwrap()["tool_result"],
        editor
    );
    assert!(review(&studio, "critic", "go").await.is_err());
    for (thread, call) in [
        ("critic", "run"),
        ("critic", "edit-critic"),
        ("unknown", "edit-unknown"),
    ] {
        let response = review(&studio, thread, call).await.unwrap();
        assert_eq!(response["success"], false, "{response}");
        assert!(
            response["tool_result"]["error"]
                .as_str()
                .unwrap()
                .contains("not authorized")
        );
    }
    assert_eq!(
        review(&studio, "author", "edit").await.unwrap()["success"],
        true
    );
    assert!(root.path().join("accepted.vr").is_file());
    assert!(
        studio.call("agent.request", author).await.is_err(),
        "recycled ID"
    );
    assert!(review(&studio, "author", "stale").await.is_err());
    assert!(!root.path().join("forbidden.vr").exists());
    assert_eq!(
        studio.call("operation.list", json!({})).await.unwrap(),
        json!([])
    );
}
