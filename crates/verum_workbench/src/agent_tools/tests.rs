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
        "verum_lint",
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
    let connection = provider.events.lock().await.connection().to_string();
    *studio.agent.lock().await = Some(Arc::new(provider));
    let author = json!({"method":"thread/start","connectionId":connection,"params":{}});
    let mut stale = author.clone();
    stale["connectionId"] = json!("retired");
    assert!(studio.call("agent.request", stale).await.unwrap_err().to_string().contains("connection changed"));
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

#[tokio::test]
async fn packet_binds_one_declaration_and_retained_proof_without_promoting_the_run_scope() {
    let root = tempfile::tempdir().unwrap();
    let source =
        "fn first(x: Bool) -> Bool ensures result == x { x }\nfn second(x: Bool) -> Bool { x }\n";
    let hash = crate::workspace::digest(source.as_bytes());
    for path in ["subject.vr", "other.vr"] {
        std::fs::write(root.path().join(path), source).unwrap();
    }
    let request = verum_source_proof::SourceRequest {
        profile: verum_source_proof::SourceProfile::Finite.as_str().into(),
        source: source.into(),
        declaration: "first".into(),
    };
    let proof = verum_source_proof::replay(&serde_json::to_vec(&request).unwrap());
    assert_eq!(
        proof.validate_for(&request).unwrap(),
        verum_source_proof::SourceOutcome::Verified
    );
    let run = root.path().join(".verum-studio/runs/saved-proof");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(run.join("source.vr"), source).unwrap();
    std::fs::write(run.join("receipt.json"), serde_json::to_vec(&json!({
        "id":"saved-proof","kind":"proof","path":"subject.vr","source_hash":hash,"source_mode":"source",
        "declaration":"first","proof_profile":request.profile,"status":"succeeded","proof":proof,
        "executable":"verum-source-replay","timeout_ms":1000,"elapsed_ms":1,"exit_code":0,
        "stdout":"","stderr":"","evidence_path":run,"started_unix_ms":1,"termination":"exit","version":1
    })).unwrap()).unwrap();
    for (id, mode) in [("legacy-proof", None), ("script-proof", Some("script"))] {
        let mut receipt: Value = serde_json::from_slice(&std::fs::read(run.join("receipt.json")).unwrap()).unwrap();
        receipt["id"] = json!(id);
        if let Some(mode) = mode { receipt["source_mode"] = json!(mode); }
        else { receipt.as_object_mut().unwrap().remove("source_mode"); }
        let directory = root.path().join(".verum-studio/runs").join(id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("source.vr"), source).unwrap();
        std::fs::write(directory.join("receipt.json"), serde_json::to_vec(&receipt).unwrap()).unwrap();
    }
    let studio = Studio::open(Config {
        workspace: root.path().into(),
        codex: "unused".into(),
        verum: "unused".into(),
        proof: "unused".into(),
        port: 0,
    })
    .unwrap();
    let raw = studio
        .call("language.request", json!({"path":"subject.vr"}))
        .await
        .unwrap();
    let snapshot = &raw["result"];
    let first = &snapshot["declarations"][0];
    let args = json!({"path":"subject.vr","expected_source_hash":hash,"expected_project_revision":snapshot["project_revision"],
        "declaration_id":first["id"],"operation_id":"saved-proof"});
    let packet = studio.call("evidence.packet", args.clone()).await.unwrap();
    assert_eq!(
        packet,
        studio
            .tool(&json!({"tool":"verum_evidence","arguments":args}))
            .await
            .unwrap()
    );
    assert_eq!(packet["schema"], "verum-evidence-packet/1");
    assert_eq!(packet["analysis"]["declaration"], *first);
    assert_eq!(
        packet["analysis"]["project_revision"],
        snapshot["project_revision"]
    );
    assert_eq!(
        packet["analysis"]["document_diagnostics"]["total"],
        snapshot["diagnostics"].as_array().unwrap().len()
    );
    assert_eq!(
        packet["analysis"]["declaration_span"],
        json!(proof.source.span)
    );
    assert_eq!(packet["operation"]["relation"], "checker-source-span");
    assert_eq!(packet["operation"]["status"], "succeeded");
    assert_eq!(
        packet["operation"].get("project_revision"),
        Some(&Value::Null)
    );
    assert!(packet["operation"].get("stdout").is_none());

    for (field, invalid) in [
        ("expected_source_hash", json!("0".repeat(64))),
        ("expected_project_revision", json!("unrelated revision")),
        ("declaration_id", snapshot["declarations"][1]["id"].clone()),
        ("declaration_id", json!("unknown")),
        ("operation_id", json!("missing")),
        ("operation_id", json!("legacy-proof")),
        ("operation_id", json!("script-proof")),
        ("source_mode", json!("script")),
    ] {
        let mut wrong = args.clone();
        wrong[field] = invalid;
        assert!(
            studio.call("evidence.packet", wrong).await.is_err(),
            "{field}"
        );
    }
    // A cold request with a stale selector closes its temporary LSP document too.
    assert!(
        studio
            .call(
                "evidence.packet",
                json!({"path":"other.vr","expected_source_hash":hash,"declaration_id":"missing"})
            )
            .await
            .is_err()
    );
    let changed = "fn another() -> Int { 1 }";
    std::fs::write(root.path().join("other.vr"), changed).unwrap();
    let cold = studio.call("evidence.packet",json!({"path":"other.vr","expected_source_hash":crate::workspace::digest(changed.as_bytes())})).await.unwrap();
    assert_eq!(
        cold["analysis"]["source_hash"],
        crate::workspace::digest(changed.as_bytes()).as_str()
    );
    // The subject is byte-for-byte unchanged, but its captured project has changed.
    assert!(snapshot["project_revision"].is_string());
    assert_ne!(cold["analysis"]["project_revision"], snapshot["project_revision"]);
    assert!(studio.call("evidence.packet", args.clone()).await.is_err());
    std::fs::write(root.path().join("other.vr"), source).unwrap();
    // Identical bytes in another file cannot acquire this proof by a document join.
    assert!(
        studio
            .call(
                "evidence.packet",
                json!({"path":"other.vr","expected_source_hash":hash,"operation_id":"saved-proof"})
            )
            .await
            .is_err()
    );
    let document = studio
        .call(
            "evidence.packet",
            json!({"path":"subject.vr","expected_source_hash":hash,"operation_id":"saved-proof"}),
        )
        .await
        .unwrap();
    assert_eq!(document["operation"]["relation"], "input-source");
    assert_eq!(document["analysis"]["declaration"], Value::Null);

    let draft = "fn first(x: Bool) -> Bool { 99 }";
    let active = studio
        .call(
            "language.request",
            json!({"path":"subject.vr","source":draft}),
        )
        .await
        .unwrap();
    assert!(studio.call("evidence.packet", args).await.is_err());
    let pinned = studio.call("evidence.packet",json!({"path":"subject.vr","source":draft,"expected_source_hash":active["source_hash"]})).await.unwrap();
    assert_eq!(pinned["analysis"]["version"], active["version"]);
    assert!(
        pinned["analysis"]["document_diagnostics"]["errors"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(pinned["operation"], Value::Null);
}
