use super::*;
use crate::agent_tools::Role;

fn endpoint(events: &Events) -> Address {
    Address {
        connection_id: events.connection(),
        thread_id: "local-id".into(),
    }
}

fn completed(text: &str) -> Value {
    json!({"method":"item/completed","params":{"threadId":"local-id","turnId":"turn-2",
          "item":{"type":"agentMessage","id":"item-3","text":text}}})
}

fn start(events: &mut Events, thread: &str) {
    events
        .push(json!({"method":"turn/started","params":{"threadId":thread,"turn":{"id":"turn-2"}}}))
        .unwrap();
}

fn live() -> Events {
    let mut events = Events::default();
    events.register_thread("local-id", Role::Reviewer).unwrap();
    start(&mut events, "local-id");
    events
}

#[test]
fn only_host_observed_completed_messages_in_registered_conversations_can_be_shared() {
    let mut source = Events::default();
    let target = live();
    let mut selection = Selection {
        from: endpoint(&source),
        to: endpoint(&target),
        seq: 1,
    };
    source.push(completed("early")).unwrap();
    source.register_thread("local-id", Role::Author).unwrap();
    assert!(Message::observe(&source, &target, &selection).is_err());
    let mut request = completed("spoofed");
    request["id"] = json!(42);
    source.push(request).unwrap();
    selection.seq = 2;
    assert!(Message::observe(&source, &target, &selection).is_err());
    let mut incomplete = completed("partial");
    incomplete["method"] = json!("item/agentMessage/delta");
    source.push(incomplete).unwrap();
    selection.seq = 3;
    assert!(Message::observe(&source, &target, &selection).is_err());
    source.push(completed("unstarted")).unwrap();
    selection.seq = 4;
    assert!(Message::observe(&source, &target, &selection).is_err());
    start(&mut source, "local-id");
    source.push(completed("λ\nданные")).unwrap();
    selection.seq = 6;
    let message = Message::observe(&source, &target, &selection).unwrap();
    assert_eq!(message.sha256, digest("λ\nданные".as_bytes()));
    assert_eq!(message.turn_id, "turn-2");
    assert_eq!(message.item_id, "item-3");
    assert_ne!(message.from.connection_id, message.to.connection_id);
    assert!(Message::observe(&source, &source, &selection).is_err());
    source.push(json!({"method":"turn/completed","params":{"threadId":"local-id","turn":{"id":"turn-2"}}})).unwrap();
    start(&mut source, "local-id");
    source.push(completed("late")).unwrap();
    selection.seq = 9;
    assert!(Message::observe(&source, &target, &selection).is_err());
    selection.seq = 6;
    // Reusing the same native ID in this connection permanently retires it.
    assert!(source.register_thread("local-id", Role::Author).is_err());
    assert!(Message::observe(&source, &target, &selection).is_err());
}

#[test]
fn immutable_addressed_messages_survive_reopen_and_duplicate_writers() {
    let root = tempfile::tempdir().unwrap();
    let mut source = live();
    let target = live();
    let third = live();
    let selection = Selection {
        from: endpoint(&source),
        to: endpoint(&target),
        seq: 2,
    };
    source
        .push(completed("A verified result needs its receipt"))
        .unwrap();
    let msg = Message::observe(&source, &target, &selection).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| msg.deliver(root.path()).unwrap());
        }
    });
    let dir = root.path().to_path_buf();
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    assert_eq!(
        page(&dir, Visibility::Recipient(msg.to.clone()), None).unwrap()["messages"][0],
        json!(msg)
    );
    assert_eq!(
        page(&dir, Visibility::Participant(msg.from.clone()), None).unwrap()["messages"][0],
        json!(msg)
    );
    assert_eq!(
        page(&dir, Visibility::Recipient(msg.from.clone()), None).unwrap()["messages"],
        json!([])
    );
    assert_eq!(
        page(&dir, Visibility::Participant(endpoint(&third)), None).unwrap()["messages"],
        json!([])
    );
    let path = dir.join(msg.filename().unwrap());
    let mut changed = msg.clone();
    changed.text = "changed on disk".into();
    std::fs::write(path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(page(&dir, Visibility::Recipient(msg.to.clone()), None).is_err());
    assert!(msg.deliver(&dir).is_err());
}

#[test]
fn mailbox_pages_cover_each_addressed_message_once() {
    let root = tempfile::tempdir().unwrap();
    let mut source = live();
    let target = live();
    let from = endpoint(&source);
    let to = endpoint(&target);
    let mut deliver = |seq| {
        source.push(completed(&format!("turn {seq}"))).unwrap();
        let packet = Message::observe(
            &source,
            &target,
            &Selection {
                from: from.clone(),
                to: to.clone(),
                seq,
            },
        )
        .unwrap();
        packet.deliver(root.path()).unwrap();
        std::fs::File::open(root.path().join(packet.filename().unwrap()))
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(seq))
            .unwrap();
    };
    for seq in 2..=(PAGE as u64 + 3) {
        deliver(seq);
    }
    let first = page(root.path(), Visibility::Recipient(to.clone()), None).unwrap();
    assert_eq!(first["messages"][0]["seq"], PAGE as u64 + 3);
    deliver(PAGE as u64 + 4);
    assert_eq!(
        page(root.path(), Visibility::Recipient(to.clone()), None).unwrap()["messages"][0]["seq"],
        PAGE as u64 + 4
    );
    let second = page(
        root.path(),
        Visibility::Recipient(to.clone()),
        first["next"].as_str(),
    )
    .unwrap();
    assert_eq!(first["messages"].as_array().unwrap().len(), PAGE);
    assert_eq!(second["messages"].as_array().unwrap().len(), 2);
    assert!(second["next"].is_null());
    let ids = [&first, &second]
        .into_iter()
        .flat_map(|p| p["messages"].as_array().unwrap())
        .map(|m| m["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), PAGE + 2);
}

#[cfg(unix)]
#[tokio::test]
async fn operator_and_reviewed_agent_use_the_same_immutable_delivery() {
    use crate::{Config, codex::Codex};
    use std::{fs, os::unix::fs::PermissionsExt, sync::Arc};

    let work = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let binary = data.path().join("provider");
    fs::write(
        &binary,
        r#"#!/bin/sh
while IFS= read -r line; do
    case "$line" in
        *'"method":"initialize"'*) echo '{"id":1,"result":{}}' ;;
        *'"method":"thread/start"'*|*'"method":"initialized"'*) ;;
    esac
done
"#,
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let open = || {
        let mut studio = Studio::open(Config {
            workspace: work.path().into(),
            verum: "unused".into(),
            proof: "unused".into(),
            codex: binary.clone(),
            port: 0,
        })
        .unwrap();
        studio.handoffs.data_root = Some(data.path().to_owned());
        studio
    };
    let studio = open();
    let provider = Arc::new(
        Codex::connect_with_storage(&binary, work.path(), data.path())
            .await
            .unwrap(),
    );
    let connection = provider.events.lock().await.connection();
    *studio.agent.lock().await = Some(provider.clone());
    let from = json!({"threadId":"author","connectionId":connection});
    let to = json!({"threadId":"recipient","connectionId":connection});
    {
        let mut events = provider.events.lock().await;
        events.register_thread("author", Role::Author).unwrap();
        events.register_thread("recipient", Role::Reviewer).unwrap();
        start(&mut events, "author");
        let mut finished = completed("exact bytes Ω");
        finished["params"]["threadId"] = json!("author");
        events.push(finished).unwrap();
    }

    let packet = studio
        .call(
            "agent.handoff.preview",
            json!({"from":from,"to":to,"seq":2}),
        )
        .await
        .unwrap();
    assert!(
        studio
            .call(
                "agent.handoff.send",
                json!({"id":packet["id"],"text":"replaced"})
            )
            .await
            .is_err()
    );
    assert_eq!(
        studio.call("agent.handoff.list", to.clone()).await.unwrap()["messages"],
        json!([])
    );

    let respond = |approval: Value, owner: &str, allowed| {
        let args = json!({"connectionId":connection,"threadId":owner,"reviewId":approval,"result":{"approved":allowed}});
        studio.call("agent.respond", args)
    };
    for (wire, approve) in [("first", false), ("second", true)] {
        let mut events = provider.events.lock().await;
        events
            .push(json!({"id":wire,"method":"item/tool/call","params":{
            "threadId":"author","turnId":"turn-9","tool":"verum_handoff",
            "arguments":{"connectionId":connection,"threadId":"recipient","seq":2}}}))
            .unwrap();
        let key = events.read(0, Some("author"))["pendingReviews"][0].clone();
        drop(events);
        let reply = respond(key, "author", approve).await.unwrap();
        assert_eq!(reply["success"], approve, "{reply}");
        let listed = studio.call("agent.handoff.list", to.clone()).await.unwrap();
        assert_eq!(
            listed["messages"].as_array().unwrap().len(),
            usize::from(approve)
        );
    }

    assert_eq!(
        studio
            .call("agent.handoff.send", json!({"id":packet["id"]}))
            .await
            .unwrap(),
        packet
    );
    assert!(
        studio
            .call("agent.handoff.send", json!({"id":packet["id"]}))
            .await
            .is_err()
    );
    let mut events = provider.events.lock().await;
    events
        .push(json!({"id":"inspect","method":"item/tool/call","params":{
        "threadId":"recipient","turnId":"turn-other","tool":"verum_inbox","arguments":{}}}))
        .unwrap();
    let key = events.read(0, Some("recipient"))["pendingReviews"][0].clone();
    drop(events);
    let read = respond(key, "recipient", true).await.unwrap();
    assert_eq!(read["tool_result"]["messages"], json!([packet]));
    assert!(studio.call("agent.inbox", from.clone()).await.is_err());
    let restored = open();
    assert_eq!(
        restored
            .call("agent.handoff.history", json!({}))
            .await
            .unwrap()["messages"],
        json!([packet])
    );
    assert_eq!(
        restored.call("agent.handoff.list", to).await.unwrap()["messages"],
        json!([packet])
    );
}
